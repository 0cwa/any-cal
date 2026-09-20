package org.anycal.android.contacts

import android.provider.ContactsContract
import org.anycal.android.BridgeDecision
import org.anycal.android.BridgeRequest
import org.anycal.android.BridgeResponse
import org.anycal.android.RustSyncBridge
import org.anycal.android.sync.ProjectionBinding
import org.anycal.android.sync.ProjectionBindingStore
import org.anycal.android.sync.ProjectionDecision
import org.anycal.android.sync.ProjectionDecisions

data class ContactProviderObservation(val rowId: Long?, val hash: String?, val present: Boolean)

data class ContactProviderMutation(val rowId: Long?, val observedHash: String?)

/** Narrow boundary around ContentResolver. Tests use a pure fake; production can apply
 * the mapper's operations with ContactsContract sync-adapter URIs behind this interface. */
interface ContactsProviderGateway {
    fun capabilities(): ContactsCapabilities
    fun observe(sourceId: String): ContactProviderObservation
    fun apply(operation: ContactProjectionOperation): ContactProviderMutation
}

interface ContactBindingRepository {
    fun get(canonicalId: String): ProjectionBinding?
    fun save(binding: ProjectionBinding)
}

class StoreContactBindingRepository(
    private val store: ProjectionBindingStore,
    private val accountName: String,
    private val accountType: String,
) : ContactBindingRepository {
    override fun get(canonicalId: String) =
        store.get(accountName, accountType, ContactsContract.AUTHORITY, canonicalId)

    override fun save(binding: ProjectionBinding) = store.upsert(binding)
}

data class PlannedContactOperation(
    val canonicalId: String,
    val decision: ProjectionDecision,
    val operation: ContactProjectionOperation,
)

data class ContactsSyncReport(
    val response: BridgeResponse,
    val operations: List<PlannedContactOperation>,
    val applied: Int,
)

/** Account-owned callback. It validates the bridge and provider boundary before planning;
 * state is committed only after the provider gateway reports a successful mutation. */
class ContactsSyncCallback(
    private val accountName: String,
    private val accountType: String,
    private val gateway: ContactsProviderGateway,
    private val bindings: ContactBindingRepository,
    private val bridge: RustSyncBridge,
) {
    private val mapper = ContactsProjectionMapper(accountName, accountType)

    fun sync(
        request: BridgeRequest,
        contacts: List<ContactRecord>,
        apply: Boolean = true,
        responseOverride: BridgeResponse? = null,
    ): ContactsSyncReport {
        request.validate()
        require(request.accountName == accountName && request.accountType == accountType) {
            "bridge account scope does not belong to this callback"
        }
        require(request.authority == ContactsContract.AUTHORITY) { "unsupported contacts authority" }
        val capabilities = gateway.capabilities()
        check(capabilities.writable) { "Contacts provider unavailable or permissions are missing" }
        val payloadRequest = if (contacts.isNotEmpty() && request.resourcePayloads.isEmpty()) {
            request.copy(resourcePayloads = contacts.filter { !it.deleted }.map { it.toBridgeResource() })
        } else request
        payloadRequest.validate()
        val response = (responseOverride ?: bridge.sync(payloadRequest)).validate()
        check(response.error == null) { response.error?.message ?: "bridge rejected request" }

        val byId = contacts.associateBy { it.anytypeObjectId }
        val tombstoneIds = request.tombstones.map { it.canonicalId }.toSet()
        val requestedIds = request.resources.toSet() + tombstoneIds
        check(requestedIds.all { it.isNotBlank() }) { "bridge resource IDs must not be blank" }
        check(contacts.all { it.anytypeObjectId in requestedIds }) { "contact is outside bridge resource scope" }
        val decisions = response.decisions.associateBy { it.resourceId }
        fun decisionFor(canonicalId: String): BridgeDecision {
            val resourceId = payloadRequest.resourcePayloads.firstOrNull { it.anytypeObjectId == canonicalId }?.resourceId
            val tombstoneResourceId = payloadRequest.tombstones.firstOrNull { it.canonicalId == canonicalId }?.resourceId
            return decisions[resourceId]?.decision ?: decisions[tombstoneResourceId]?.decision ?: decisions[canonicalId]?.decision
            ?: error("bridge response omitted a requested resource: $canonicalId")
        }

        val planned = requestedIds.sorted().flatMap { id ->
            val tombstone = request.tombstones.firstOrNull { it.canonicalId == id }
            val contact = if (tombstone != null) {
                require(tombstone.canonicalId.isNotBlank() && tombstone.revision > 0L) {
                    "bridge tombstone must include a canonical ID and revision"
                }
                ContactRecord(id, id, deleted = true, canonicalRevision = tombstone.revision.toString())
            } else {
                byId[id] ?: error("bridge resource has no contact payload: $id")
            }
            val bridgeDecision = decisionFor(id)
            check(bridgeDecision == BridgeDecision.UPSERT || bridgeDecision == BridgeDecision.NOOP ||
                bridgeDecision == BridgeDecision.ARCHIVE) { "bridge did not authorize contact projection: $id" }
            val source = mapper.sourceId(id)
            val binding = bindings.get(id)
            if (binding != null) {
                require(binding.accountName == accountName && binding.accountType == accountType &&
                    binding.authority == ContactsContract.AUTHORITY && binding.sourceId == source) {
                    "projection binding identity does not belong to this account"
                }
            }
            val observation = gateway.observe(source)
            val hash = if (contact.deleted) null else mapper.projectionHash(contact)
            val decision = ProjectionDecisions.classify(
                binding, hash, observation.hash, observation.present, contact.deleted,
            )
            val operations = if (decision == ProjectionDecision.NoOp ||
                decision == ProjectionDecision.ObserverEcho || decision == ProjectionDecision.Replay) {
                emptyList()
            } else mapper.plan(contact)
            operations.map { PlannedContactOperation(id, decision, it) }
        }

        var applied = 0
        if (apply) {
            planned.groupBy { it.canonicalId }.toSortedMap().forEach { (id, items) ->
                val contact = byId[id]
                val tombstone = request.tombstones.firstOrNull { it.canonicalId == id }
                var mutation = ContactProviderMutation(bindings.get(id)?.providerRowId, null)
                items.forEach { item -> mutation = gateway.apply(item.operation) }
                val source = mapper.sourceId(id)
                val deleted = contact?.deleted == true || tombstone != null
                val canonicalRevision = if (deleted) {
                    contact?.canonicalRevision?.takeIf { it.isNotBlank() }
                        ?: tombstone?.revision?.toString()
                } else {
                    contact?.canonicalRevision
                }
                bindings.save(ProjectionBinding(
                    accountName, accountType, ContactsContract.AUTHORITY, id,
                    mutation.rowId, source,
                    if (deleted) null else contact?.let(mapper::projectionHash),
                    mutation.observedHash, canonicalRevision,
                    if (deleted) canonicalRevision else null,
                    null, mutation.observedHash,
                ))
                applied += items.size
            }
        }
        return ContactsSyncReport(response, planned, applied)
    }
}
