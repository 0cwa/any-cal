package org.anycal.android.calendar

import android.content.ContentValues
import android.provider.CalendarContract
import org.anycal.android.BridgeCheckpoint
import org.anycal.android.BridgeResponse

/** Versioned, provider-neutral input from the Rust sync boundary. */
data class CalendarBridgeRequest(
    val schemaVersion: Int,
    val accountName: String,
    val accountType: String,
    val authority: String,
    val capability: CalendarProviderCapability,
    val records: List<CalendarBridgeRecord>,
    val existing: List<CalendarProviderSnapshot> = emptyList(),
    val rebuild: Boolean = false,
    /** Published only after the provider gateway has applied this request. */
    val checkpointToCommit: BridgeCheckpoint? = null,
    val response: BridgeResponse? = null,
)

data class CalendarBridgeRecord(
    val envelope: CalendarEnvelope,
    val sourceId: String,
)

/** Snapshot supplied by the provider reader; it avoids querying ContentResolver in the planner. */
data class CalendarProviderSnapshot(
    val sourceId: String,
    val projectionHash: String?,
    val providerId: Long? = null,
    val calendarProviderId: Long? = null,
    val accountName: String,
    val accountType: String,
)

/** Provider-free validation result used by JVM tests and bridge preflight. */
data class CalendarCallbackValidation(
    val orderedSourceIds: List<String>,
    val tombstoneSourceIds: List<String>,
)

sealed interface CalendarProviderOperation {
    val sourceId: String
    data class DeleteEvent(override val sourceId: String, val providerId: Long?) : CalendarProviderOperation
    data class DeleteCalendar(override val sourceId: String, val providerId: Long?) : CalendarProviderOperation
    data class EnsureCalendar(override val sourceId: String, val values: ContentValues, val updateProviderId: Long?) : CalendarProviderOperation
    data class EnsureEvent(
        override val sourceId: String,
        val calendarSourceId: String,
        val values: ContentValues,
        val updateProviderId: Long?,
        val attendees: List<ContentValues>,
        val reminders: List<ContentValues>,
    ) : CalendarProviderOperation
    data class NoOp(override val sourceId: String) : CalendarProviderOperation
}

sealed interface CalendarCallbackResult {
    data class Planned(val operations: List<CalendarProviderOperation>) : CalendarCallbackResult
    data class Rejected(val reason: String) : CalendarCallbackResult
}

/**
 * Pure callback planner. It is intentionally not wired to AbstractThreadedSyncAdapter:
 * the adapter can later provide a bridge DTO and an applier without making planning mutable.
 */
object CalendarSyncCallback {
    const val SCHEMA_VERSION = 1

    fun validate(request: CalendarBridgeRequest): Result<CalendarCallbackValidation> {
        if (request.schemaVersion != SCHEMA_VERSION) return Result.failure(IllegalArgumentException("unsupported bridge schema"))
        if (request.authority != CalendarContract.AUTHORITY) return Result.failure(IllegalArgumentException("wrong CalendarContract authority"))
        if (request.accountName.isBlank() || request.accountType.isBlank()) return Result.failure(IllegalArgumentException("missing account identity"))
        if (!request.capability.writable) return Result.failure(IllegalArgumentException(request.capability.reason ?: "CalendarContract is not writable"))
        val snapshots = request.existing.associateBy { it.sourceId }
        if (snapshots.size != request.existing.size) return Result.failure(IllegalArgumentException("duplicate provider source identity"))
        if (request.existing.any { it.accountName != request.accountName || it.accountType != request.accountType }) return Result.failure(IllegalArgumentException("provider account ownership mismatch"))
        val records = request.records.sortedBy { it.sourceId }
        if (records.map { it.sourceId }.toSet().size != records.size) return Result.failure(IllegalArgumentException("duplicate bridge source identity"))
        records.forEach { record ->
            val expected = CalendarContractProjection.sourceId(request.accountType, request.accountName, record.envelope.canonicalId)
            val calendarExpected = CalendarContractProjection.sourceId(request.accountType, request.accountName, record.envelope.calendarCanonicalId)
            if (record.sourceId != expected) return Result.failure(IllegalArgumentException("event source identity mismatch"))
            if (record.envelope.calendarCanonicalId.isBlank() || calendarExpected.isBlank()) return Result.failure(IllegalArgumentException("calendar identity missing"))
            if (record.envelope.start.epochMillis() == null) return Result.failure(IllegalArgumentException("event start is not parseable"))
            if (record.envelope.end?.epochMillis() == null && record.envelope.end != null) return Result.failure(IllegalArgumentException("event end is not parseable"))
        }
        return Result.success(CalendarCallbackValidation(records.map { it.sourceId }, records.filter { it.envelope.deleted }.map { it.sourceId }))
    }

    fun plan(request: CalendarBridgeRequest): CalendarCallbackResult {
        validate(request).onFailure { return CalendarCallbackResult.Rejected(it.message ?: "invalid bridge request") }

        val snapshots = request.existing.associateBy { it.sourceId }
        val records = request.records.sortedBy { it.sourceId }

        val plans = mutableListOf<Pair<String, CalendarProjectionPlan?>>()
        records.forEach { record ->
            val expected = CalendarContractProjection.sourceId(request.accountType, request.accountName, record.envelope.canonicalId)
            if (record.sourceId != expected) return CalendarCallbackResult.Rejected("event source identity mismatch")
            val calendarExpected = CalendarContractProjection.sourceId(request.accountType, request.accountName, record.envelope.calendarCanonicalId)
            val projection = when (val result = CalendarContractProjection.plan(record.envelope, request.accountType, request.accountName, request.capability)) {
                is CalendarProjectionResult.Unsupported -> return CalendarCallbackResult.Rejected(result.reason)
                is CalendarProjectionResult.Planned -> result.plan
            }
            plans += record.sourceId to projection
            if (projection.eventSourceId != expected || projection.calendarSourceId != calendarExpected) {
                return CalendarCallbackResult.Rejected("projection identity mismatch")
            }
        }

        val deletes = mutableListOf<CalendarProviderOperation>()
        val calendars = mutableListOf<CalendarProviderOperation>()
        val events = mutableListOf<CalendarProviderOperation>()
        plans.forEach { (sourceId, plan) ->
            val snapshot = snapshots[sourceId]
            if (plan!!.tombstone) {
                deletes += CalendarProviderOperation.DeleteEvent(sourceId, snapshot?.providerId)
                return@forEach
            }
            val calendarSnapshot = snapshots[plan.calendarSourceId]
            if (calendarSnapshot == null) {
                calendars += CalendarProviderOperation.EnsureCalendar(plan.calendarSourceId, plan.calendarValues, null)
            }
            if (snapshot?.projectionHash == plan.projectionHash) {
                events += CalendarProviderOperation.NoOp(sourceId)
            } else {
                events += CalendarProviderOperation.EnsureEvent(sourceId, plan.calendarSourceId, plan.eventValues!!, snapshot?.providerId,
                    plan.attendees, plan.reminders)
            }
        }
        // Tombstones first; parent calendars are never deleted before their children.
        val operations = (deletes + calendars + events).sortedWith(compareBy<CalendarProviderOperation>({ phase(it) }, { it.sourceId }))
        return CalendarCallbackResult.Planned(operations)
    }

    private fun phase(operation: CalendarProviderOperation): Int = when (operation) {
        is CalendarProviderOperation.DeleteEvent -> 10
        is CalendarProviderOperation.DeleteCalendar -> 20
        is CalendarProviderOperation.EnsureCalendar -> 30
        is CalendarProviderOperation.EnsureEvent, is CalendarProviderOperation.NoOp -> 40
    }
}

/** Narrow seam for later application; implementations may wrap CalendarContractStore. */
fun interface CalendarProviderApplier {
    fun apply(operation: CalendarProviderOperation): Result<Unit>
}
