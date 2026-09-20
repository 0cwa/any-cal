package org.anycal.android.contacts

import android.provider.ContactsContract
import org.anycal.android.BridgeDecision
import org.anycal.android.BridgeRequest
import org.anycal.android.BridgeResponse
import org.anycal.android.BridgeTombstone
import org.anycal.android.RustSyncBridge

/** Sends only account-owned provider edits back to the configured canonical
 * service. A provider query failure must happen before this class is called;
 * an empty observation is never inferred from an unavailable provider. */
class NativeContactsOutboundSource(
    private val accountName: String,
    private val accountType: String,
    private val bridge: RustSyncBridge,
    private val onResponse: (BridgeResponse) -> Unit = {},
) : ContactsOutboundSource {
    override fun publish(changes: List<ContactOutboundChange>): ContactPublishResult {
        if (changes.isEmpty()) return ContactPublishResult.Success
        val upserts = changes.filterIsInstance<ContactOutboundChange.Upsert>()
        val tombstones = changes.filterIsInstance<ContactOutboundChange.Tombstone>()
        val request = BridgeRequest(
            accountName = accountName,
            accountType = accountType,
            authority = ContactsContract.AUTHORITY,
            checkpoint = null,
            resources = upserts.map { it.canonicalId },
            tombstones = tombstones.map {
                BridgeTombstone(
                    resourceId = "contact:${it.canonicalId}",
                    canonicalId = it.canonicalId,
                    revision = 1L,
                    collectionId = "contacts",
                )
            },
            resourcePayloads = upserts.map { it.record.toBridgeResource() },
        )
        val response = bridge.sync(request).validate()
        val error = response.error
        if (error != null) {
            return when (error.code) {
                org.anycal.android.BridgeErrorCode.TRANSPORT_UNAVAILABLE -> ContactPublishResult.Retryable("bridge transport unavailable")
                else -> ContactPublishResult.Rejected("bridge rejected provider change")
            }
        }
        val decisions = response.decisions.associateBy { it.resourceId }
        val authorized = upserts.all { change ->
            val resource = change.record.toBridgeResource()
            decisions[resource.resourceId]?.decision in setOf(BridgeDecision.UPSERT, BridgeDecision.NOOP) ||
                decisions[change.canonicalId]?.decision in setOf(BridgeDecision.UPSERT, BridgeDecision.NOOP)
        } && tombstones.all { change ->
            decisions["contact:${change.canonicalId}"]?.decision in setOf(BridgeDecision.ARCHIVE, BridgeDecision.NOOP) ||
                decisions[change.canonicalId]?.decision in setOf(BridgeDecision.ARCHIVE, BridgeDecision.NOOP)
        }
        if (!authorized) return ContactPublishResult.Rejected("bridge response omitted provider-change authorization")
        onResponse(response)
        return ContactPublishResult.Success
    }
}
