package org.anycal.android.calendar

import android.provider.CalendarContract
import org.anycal.android.BridgeCheckpoint
import org.anycal.android.BridgeDocument
import org.anycal.android.BridgeOccurrence
import org.anycal.android.BridgeRequest
import org.anycal.android.BridgeResource
import org.anycal.android.BridgeResponse
import org.anycal.android.BridgeTombstone
import org.anycal.android.BridgeDecision
import org.anycal.android.RustSyncBridge
import java.time.Instant
import java.time.LocalDate
import java.time.format.DateTimeFormatter

/** Publishes account-owned CalendarContract edits through the same canonical
 * bridge used for pulls. Provider IDs never cross the bridge; Anytype object
 * IDs and DAV UIDs remain the stable identity. */
class NativeCalendarOutboundSource(
    private val accountName: String,
    private val accountType: String,
    private val bridge: RustSyncBridge,
    private val onResponse: (BridgeResponse) -> Unit = {},
) : CalendarOutboundSink {
    override fun emit(
        changes: List<CalendarOutboundChange>,
        checkpoint: CalendarReconcileCheckpoint,
    ): Result<Unit> {
        if (changes.isEmpty()) return Result.success(Unit)
        val upserts = changes.filterIsInstance<CalendarOutboundChange.Upsert>()
        val tombstones = changes.filterIsInstance<CalendarOutboundChange.Tombstone>()
        val resources = upserts.map { it.envelope.toBridgeResource() }
        val request = BridgeRequest(
            accountName = accountName,
            accountType = accountType,
            authority = CalendarContract.AUTHORITY,
            checkpoint = checkpoint.token?.let { BridgeCheckpoint(it, checkpoint.accountGeneration) },
            resources = resources.map { it.anytypeObjectId },
            tombstones = tombstones.map { change ->
                val canonicalId = change.sourceId.canonicalId(accountType, accountName)
                BridgeTombstone(
                    resourceId = "event:$canonicalId",
                    canonicalId = canonicalId,
                    revision = 1L,
                    collectionId = "tasks",
                )
            },
            resourcePayloads = resources,
        )
        val response = runCatching { bridge.sync(request).validate() }
            .getOrElse { return Result.failure(it) }
        response.error?.let { return Result.failure(IllegalStateException("calendar bridge rejected change")) }
        val decisions = response.decisions.associateBy { it.resourceId }
        upserts.forEach { change ->
            val resource = change.envelope.toBridgeResource()
            check(decisions[resource.resourceId]?.decision in setOf(BridgeDecision.UPSERT, BridgeDecision.NOOP) ||
                decisions[resource.anytypeObjectId]?.decision in setOf(BridgeDecision.UPSERT, BridgeDecision.NOOP)) {
                "bridge response omitted calendar edit authorization"
            }
        }
        tombstones.forEach { change ->
            val canonicalId = change.sourceId.canonicalId(accountType, accountName)
            val resourceId = "event:$canonicalId"
            check(decisions[resourceId]?.decision in setOf(BridgeDecision.ARCHIVE, BridgeDecision.NOOP) ||
                decisions[canonicalId]?.decision in setOf(BridgeDecision.ARCHIVE, BridgeDecision.NOOP)) {
                "bridge response omitted calendar tombstone authorization"
            }
        }
        onResponse(response)
        return Result.success(Unit)
    }
}

private fun CalendarEnvelope.toBridgeResource(): BridgeResource {
    val fields = linkedMapOf<String, List<BridgeOccurrence>>()
    fun put(name: String, value: String?, params: Map<String, List<String>> = emptyMap()) {
        if (!value.isNullOrBlank()) fields[name] = listOf(BridgeOccurrence(value, params))
    }
    fun putMany(name: String, values: List<String>) {
        if (values.isNotEmpty()) fields[name] = values.map { BridgeOccurrence(it) }
    }
    put("UID", davUid)
    put("SUMMARY", title)
    put("DESCRIPTION", description)
    put("LOCATION", location)
    put("DTSTART", encodeDateTime(start), start.timezone?.let { mapOf("TZID" to listOf(it)) } ?: emptyMap())
    end?.let { value ->
        put("DTEND", encodeDateTime(value), value.timezone?.let { mapOf("TZID" to listOf(it)) } ?: emptyMap())
    }
    put("DURATION", duration)
    put("STATUS", statusName(status))
    put("TRANSP", transparencyName(transparency))
    put("RRULE", recurrenceRule)
    putMany("RDATE", recurrenceDates)
    putMany("EXDATE", exceptionDates)
    if (attendees.isNotEmpty()) {
        fields["ATTENDEE"] = attendees.map { attendee ->
            BridgeOccurrence(
                value = if (attendee.email.startsWith("mailto:", ignoreCase = true)) attendee.email else "mailto:${attendee.email}",
                params = buildMap {
                    attendee.name?.let { put("CN", listOf(it)) }
                    put("ROLE", listOf(roleName(attendee.type)))
                    put("PARTSTAT", listOf(participationName(attendee.status)))
                },
            )
        }
    }
    opaque.sortedBy { it.order }.forEach { property ->
        fields[property.name] = (fields[property.name].orEmpty() + BridgeOccurrence(property.value, property.parameters))
    }
    return BridgeResource(
        collectionId = "tasks",
        resourceId = "event:$canonicalId",
        kind = "event",
        anytypeObjectId = canonicalId,
        davUid = davUid,
        document = BridgeDocument(fields),
        revision = revision.toLongOrNull() ?: 0L,
    ).validate()
}

private fun String.canonicalId(accountType: String, accountName: String): String {
    val prefix = "android/$accountType/$accountName/"
    return removePrefix(prefix).takeIf { it.isNotBlank() } ?: error("calendar source identity is invalid")
}

private fun encodeDateTime(value: CalendarDateTime): String {
    if (value.allDay) {
        return runCatching { LocalDate.parse(value.value.take(10)).format(DateTimeFormatter.BASIC_ISO_DATE) }
            .getOrElse { value.value }
    }
    return runCatching { Instant.parse(value.value).toString() }
        .getOrElse { value.value }
}

private fun statusName(value: Int?): String? = when (value) {
    CalendarContract.Events.STATUS_TENTATIVE -> "TENTATIVE"
    CalendarContract.Events.STATUS_CONFIRMED -> "CONFIRMED"
    CalendarContract.Events.STATUS_CANCELED -> "CANCELLED"
    else -> null
}

private fun transparencyName(value: Int?): String? = when (value) {
    CalendarContract.Events.TRANSP_TRANSPARENT -> "TRANSPARENT"
    CalendarContract.Events.TRANSP_OPAQUE -> "OPAQUE"
    else -> null
}

private fun roleName(value: Int): String = when (value) {
    CalendarContract.Attendees.TYPE_OPTIONAL -> "OPT-PARTICIPANT"
    CalendarContract.Attendees.TYPE_RESOURCE -> "NON-PARTICIPANT"
    else -> "REQ-PARTICIPANT"
}

private fun participationName(value: Int): String = when (value) {
    CalendarContract.Attendees.ATTENDEE_STATUS_ACCEPTED -> "ACCEPTED"
    CalendarContract.Attendees.ATTENDEE_STATUS_DECLINED -> "DECLINED"
    CalendarContract.Attendees.ATTENDEE_STATUS_TENTATIVE -> "TENTATIVE"
    else -> "NEEDS-ACTION"
}
