package org.anycal.android.calendar

import android.accounts.AccountManager
import android.content.Context
import android.provider.CalendarContract
import org.anycal.android.BridgeResponse
import org.anycal.android.BridgeRequest
import org.anycal.android.RustSyncBridge
import org.anycal.android.account.ANYCAL_ACCOUNT_GENERATION_KEY
import org.anycal.android.sync.ProjectionBinding
import org.anycal.android.sync.ProjectionBindingStore

/** Bridge handoff; unavailable means fail closed rather than deleting provider data. */
fun interface CalendarBridgeSource {
    fun request(accountName: String, accountType: String, authority: String): Result<CalendarBridgeRequest>
}

object UnavailableCalendarBridgeSource : CalendarBridgeSource {
    override fun request(accountName: String, accountType: String, authority: String) =
        Result.failure<CalendarBridgeRequest>(IllegalStateException("calendar bridge unavailable"))
}

class CalendarSyncRunner(
    private val context: Context,
    private val source: CalendarBridgeSource = UnavailableCalendarBridgeSource,
    private val onCheckpointCommitted: (org.anycal.android.BridgeCheckpoint) -> Unit = {},
    private val outboundBridge: RustSyncBridge? = null,
) {
    fun run(accountName: String, accountType: String, authority: String): Result<Unit> {
        val capability = CalendarContractCapability.probe(context)
        if (!capability.writable) return Result.failure(IllegalStateException(capability.reason ?: "Calendar provider unavailable"))
        val request = source.request(accountName, accountType, authority).getOrElse { return Result.failure(it) }
        val planned = when (val result = CalendarSyncCallback.plan(request)) {
            is CalendarCallbackResult.Rejected -> return Result.failure(IllegalArgumentException(result.reason))
            is CalendarCallbackResult.Planned -> result.operations
        }
        return when (val result = CalendarContractGateway.from(context.contentResolver, accountName, accountType, capability).apply(planned)) {
            CalendarGatewayResult.Applied -> {
                val store = ProjectionBindingStore(context)
                val generation = AccountManager.get(context).getUserData(
                    android.accounts.Account(accountName, accountType),
                    ANYCAL_ACCOUNT_GENERATION_KEY,
                ) ?: "unbound"
                runCatching {
                    savePullBindings(store, accountName, accountType, request)
                    publishProviderEdits(store, accountName, accountType, generation)
                }.fold(
                    onSuccess = {
                        request.checkpointToCommit?.let(onCheckpointCommitted)
                        Result.success(Unit)
                    },
                    onFailure = { Result.failure(it) },
                )
            }
            is CalendarGatewayResult.Failed -> Result.failure(IllegalStateException(result.reason))
        }
    }

    private fun savePullBindings(
        store: ProjectionBindingStore,
        accountName: String,
        accountType: String,
        request: CalendarBridgeRequest,
    ) {
        val reader = ContentResolverCalendarReader(context, accountName, accountType)
        val sources = request.records.map { it.sourceId }.toSet()
        val snapshots = reader.snapshots(sources).associateBy { it.sourceId }
        request.records.forEach { record ->
            val envelope = record.envelope
            val existing = store.get(
                accountName,
                accountType,
                CalendarContract.AUTHORITY,
                envelope.canonicalId,
            )
            if (envelope.deleted) {
                store.upsert(
                    ProjectionBinding(
                        accountName = accountName,
                        accountType = accountType,
                        authority = CalendarContract.AUTHORITY,
                        canonicalId = envelope.canonicalId,
                        providerRowId = null,
                        sourceId = record.sourceId,
                        projectedHash = null,
                        observedHash = null,
                        canonicalRevision = envelope.revision,
                        tombstoneRevision = envelope.revision,
                        lastOperationId = null,
                        lastOperationPostHash = null,
                    ),
                )
                return@forEach
            }
            val snapshot = snapshots[record.sourceId]
                ?: error("CalendarContract omitted applied event binding")
            val hash = CalendarContractProjection.projectionHash(envelope)
            store.upsert(
                ProjectionBinding(
                    accountName = accountName,
                    accountType = accountType,
                    authority = CalendarContract.AUTHORITY,
                    canonicalId = envelope.canonicalId,
                    providerRowId = snapshot.providerId,
                    sourceId = record.sourceId,
                    projectedHash = hash,
                    observedHash = hash,
                    canonicalRevision = envelope.revision,
                    tombstoneRevision = null,
                    lastOperationId = null,
                    lastOperationPostHash = null,
                ),
            )
            // A provider may have recreated the row while preserving _SYNC_ID;
            // the fresh binding is authoritative after a successful apply.
            check(existing == null || existing.sourceId == record.sourceId) {
                "calendar binding identity changed"
            }
        }
    }

    private fun publishProviderEdits(
        store: ProjectionBindingStore,
        accountName: String,
        accountType: String,
        generation: String,
    ) {
        val bindings = store.bindings(accountName, accountType, CalendarContract.AUTHORITY)
        if (bindings.isEmpty()) return
        val reader = ContentResolverCalendarReader(context, accountName, accountType)
        val batch = reader.ownedEventBatch(bindings, 500).getOrElse { throw it }
        val generationNumber = generation.toLongOrNull() ?: generation.hashCode().toLong()
        val checkpoint = store.checkpoint(accountName, accountType, CalendarContract.AUTHORITY, generation)
        val reconciled = CalendarBidirectionalReconciler(500).reconcile(
            rows = batch.rows,
            bindings = bindings.map {
                CalendarBinding(
                    sourceId = it.sourceId,
                    lastProjectedHash = it.projectedHash,
                    lastOperationId = it.lastOperationId,
                    accountGeneration = generationNumber,
                )
            },
            checkpoint = CalendarReconcileCheckpoint(checkpoint?.cursor, generationNumber),
            nextToken = batch.nextToken,
            accountGeneration = generationNumber,
        )
        val applied = when (reconciled) {
            is CalendarReconcileResult.Rejected -> throw IllegalStateException(reconciled.reason)
            is CalendarReconcileResult.Applied -> reconciled
        }
        if (applied.changes.isEmpty()) return
        var bridgeResponse: BridgeResponse? = null
        val bridge = outboundBridge ?: return
        NativeCalendarOutboundSource(accountName, accountType, bridge, { response ->
            bridgeResponse = response
        }).emit(applied.changes, applied.checkpoint).getOrElse { throw it }
        val fresh = reader.snapshots(applied.changes.map { change ->
            when (change) {
                is CalendarOutboundChange.Upsert -> change.sourceId
                is CalendarOutboundChange.Tombstone -> change.sourceId
            }
        }.toSet()).associateBy { it.sourceId }
        applied.changes.forEach { change ->
            when (change) {
                is CalendarOutboundChange.Upsert -> {
                    val snapshot = fresh[change.sourceId]
                        ?: error("CalendarContract edit binding disappeared")
                    val hash = CalendarContractProjection.projectionHash(change.envelope)
                    store.upsert(
                        ProjectionBinding(
                            accountName, accountType, CalendarContract.AUTHORITY,
                            change.envelope.canonicalId, snapshot.providerId, change.sourceId,
                            hash, hash, change.envelope.revision, null, null, null,
                        ),
                    )
                }
                is CalendarOutboundChange.Tombstone -> {
                    val binding = bindings.firstOrNull { it.sourceId == change.sourceId }
                        ?: error("calendar tombstone binding disappeared")
                    val revision = (binding.canonicalRevision?.toLongOrNull() ?: 0L).plus(1L).toString()
                    store.upsert(binding.copy(
                        providerRowId = null,
                        projectedHash = null,
                        observedHash = null,
                        canonicalRevision = revision,
                        tombstoneRevision = revision,
                    ))
                }
            }
        }
        bridgeResponse?.checkpoint?.let(onCheckpointCommitted)
    }
}

/** Fetches canonical event envelopes through the configured bridge. It never
 * treats a failed or unavailable bridge as an empty calendar. */
class NativeCalendarBridgeSource(
    private val context: Context,
    private val bridge: RustSyncBridge,
) : CalendarBridgeSource {
    override fun request(accountName: String, accountType: String, authority: String): Result<CalendarBridgeRequest> = runCatching {
        require(authority == CalendarContract.AUTHORITY) { "unsupported calendar authority" }
        val store = ProjectionBindingStore(context)
        val generation = AccountManager.get(context).getUserData(
            android.accounts.Account(accountName, accountType),
            ANYCAL_ACCOUNT_GENERATION_KEY,
        ) ?: "unbound"
        val checkpoint = store.checkpoint(accountName, accountType, CalendarContract.AUTHORITY, generation)
        val response = bridge.sync(
            BridgeRequest(accountName, accountType, authority, checkpoint, emptyList(), emptyList()),
        ).validate()
        check(response.error == null) { response.error?.message ?: "bridge calendar pull failed" }
        val activeRecords = response.resources.filter { it.kind == "event" }.map { resource ->
            CalendarBridgeRecord(resource.toCalendarEnvelope(), resource.sourceId(accountType, accountName))
        }
        val deletedRecords = response.tombstones.map { tombstone ->
            val calendarId = tombstone.collectionId ?: "calendar"
            CalendarBridgeRecord(
                CalendarEnvelope(
                    canonicalId = tombstone.canonicalId,
                    davUid = tombstone.canonicalId,
                    calendarCanonicalId = calendarId,
                    calendarName = calendarId,
                    title = "",
                    start = CalendarDateTime("1970-01-01T00:00:00Z"),
                    revision = tombstone.revision.toString(),
                    deleted = true,
                ),
                CalendarContractProjection.sourceId(accountType, accountName, tombstone.canonicalId),
            )
        }
        val records = activeRecords + deletedRecords
        val decisions = response.decisions.associateBy { it.resourceId }
        response.resources.forEach { resource ->
            val decision = decisions[resource.resourceId]?.decision
                ?: decisions[resource.anytypeObjectId]?.decision
                ?: error("bridge response omitted calendar decision")
            check(decision == org.anycal.android.BridgeDecision.UPSERT ||
                decision == org.anycal.android.BridgeDecision.NOOP ||
                decision == org.anycal.android.BridgeDecision.ARCHIVE) {
                "bridge did not authorize calendar projection"
            }
        }
        response.tombstones.forEach { tombstone ->
            val decision = decisions[tombstone.resourceId]?.decision
                ?: decisions[tombstone.canonicalId]?.decision
                ?: error("bridge response omitted calendar tombstone decision")
            check(decision == org.anycal.android.BridgeDecision.ARCHIVE || decision == org.anycal.android.BridgeDecision.NOOP) {
                "bridge did not authorize calendar tombstone"
            }
        }
        val sourceIds = records.flatMap { record ->
            listOf(record.sourceId, CalendarContractProjection.sourceId(accountType, accountName, record.envelope.calendarCanonicalId))
        }.toSet()
        val existing = ContentResolverCalendarReader(context, accountName, accountType).snapshots(sourceIds)
        CalendarBridgeRequest(
            schemaVersion = CalendarSyncCallback.SCHEMA_VERSION,
            accountName = accountName,
            accountType = accountType,
            authority = authority,
            capability = CalendarContractCapability.probe(context),
            records = records,
            existing = existing,
            checkpointToCommit = response.checkpoint,
            response = response,
        )
    }
}

private fun org.anycal.android.BridgeResource.sourceId(accountType: String, accountName: String): String =
    CalendarContractProjection.sourceId(accountType, accountName, anytypeObjectId)
