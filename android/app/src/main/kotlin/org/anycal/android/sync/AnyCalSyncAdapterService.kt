package org.anycal.android.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.Context
import android.content.SyncResult
import android.os.Bundle
import android.provider.ContactsContract
import org.anycal.android.BridgeRequest
import org.anycal.android.BridgeRuntimeConfig
import org.anycal.android.RustSyncBridgeFactory
import org.anycal.android.RustSyncBridge
import org.anycal.android.account.ANYCAL_ACCOUNT_TOKEN_KEY
import org.anycal.android.contacts.ContactBindingRepository
import org.anycal.android.contacts.ContactRecord
import org.anycal.android.contacts.ContentResolverContactsGateway
import org.anycal.android.contacts.ContentResolverContactsReader
import org.anycal.android.contacts.ContactOutboundChange
import org.anycal.android.contacts.ContactsProviderGateway
import org.anycal.android.contacts.ContactsSyncCallback
import org.anycal.android.contacts.ContactsSyncReport
import org.anycal.android.contacts.NativeContactsOutboundSource
import org.anycal.android.contacts.StoreContactBindingRepository
import org.anycal.android.contacts.toContactRecord
import org.anycal.android.tasks.TasksOrgSyncFactory
import org.anycal.android.tasks.TasksOrgSyncReport

/** Account-owned Contacts scheduling hook; Anytype transport remains behind the bridge. */
class AnyCalSyncAdapterService : android.app.Service() {
    private lateinit var adapter: Adapter

    override fun onCreate() {
        super.onCreate()
        adapter = Adapter(this)
    }

    override fun onBind(intent: android.content.Intent?) = adapter.syncAdapterBinder

    companion object {
        /** Testable service seam; the bridge remains injectable for provider-free tests. */
        fun performContactsSync(
            account: Account,
            contacts: List<ContactRecord>,
            gateway: ContactsProviderGateway,
            bindings: ContactBindingRepository,
            bridge: RustSyncBridge,
        ): ContactsSyncReport = ContactsSyncCallback(
            account.name, account.type, gateway, bindings, bridge,
        ).sync(
            BridgeRequest(account.name, account.type, ContactsContract.AUTHORITY, null,
                contacts.map { it.anytypeObjectId }, emptyList()), contacts,
        )

        /** Pulls a bounded canonical batch, applies only validated account-owned
         * provider operations, then commits the server checkpoint. */
        fun performConfiguredContactsSync(
            context: Context,
            account: Account,
            bridge: RustSyncBridge,
        ): ContactsSyncReport {
            val store = ProjectionBindingStore(context)
            val generation = AccountManager.get(context).getUserData(account, org.anycal.android.account.ANYCAL_ACCOUNT_GENERATION_KEY)
                ?: error("account generation is unavailable")
            val checkpoint = store.checkpoint(account.name, account.type, ContactsContract.AUTHORITY, generation)
            val pull = BridgeRequest(
                accountName = account.name,
                accountType = account.type,
                authority = ContactsContract.AUTHORITY,
                checkpoint = checkpoint,
                resources = emptyList(),
                tombstones = emptyList(),
            )
            val response = bridge.sync(pull).validate()
            check(response.error == null) { response.error?.message ?: "bridge contact pull failed" }
            val resources = response.resources.filter { it.kind == "contact" }
            val contacts = resources.map { it.toContactRecord() }
            val applyRequest = pull.copy(
                resources = contacts.map { it.anytypeObjectId },
                tombstones = response.tombstones,
                resourcePayloads = resources,
            )
            val report = ContactsSyncCallback(
                account.name,
                account.type,
                ContentResolverContactsGateway(context, account.name, account.type),
                StoreContactBindingRepository(store, account.name, account.type),
                bridge,
            ).sync(applyRequest, contacts, apply = true, responseOverride = response)
            response.checkpoint?.let { store.saveCheckpoint(account.name, account.type, ContactsContract.AUTHORITY, generation, it) }
            publishProviderEdits(context, account, store, bridge)
            return report
        }

        /**
         * Explicit Tasks.org projection entry point for a background sync
         * callback. Tasks.org owns its provider authority, so this is kept as
         * a companion operation rather than registering a second sync adapter
         * for that authority. Callers may schedule it independently of the
         * Contacts sync lifecycle.
         */
        fun performConfiguredTasksSync(
            context: Context,
            account: Account,
            bridge: RustSyncBridge,
        ): Result<TasksOrgSyncReport> = TasksOrgSyncFactory.run(context, account, bridge)

        private fun publishProviderEdits(
            context: Context,
            account: Account,
            store: ProjectionBindingStore,
            bridge: RustSyncBridge,
        ) {
            val bindings = store.bindings(account.name, account.type, ContactsContract.AUTHORITY)
            if (bindings.isEmpty()) return
            val reader = ContentResolverContactsReader(context, account.name, account.type)
            val snapshots = bindings.sortedBy { it.sourceId }.chunked(500).flatMap { batch ->
                reader.ownedSnapshots(batch.map { it.sourceId }.toSet(), batch.size)
            }
                .associateBy { it.canonicalId }
            val changes = bindings.sortedBy { it.canonicalId }.mapNotNull { binding ->
                if (binding.tombstoneRevision != null) return@mapNotNull null
                val snapshot = snapshots[binding.canonicalId] ?: error("provider snapshot omitted owned contact")
                when {
                    snapshot.present && snapshot.record != null && snapshot.hash != binding.projectedHash ->
                        ContactOutboundChange.Upsert(binding.canonicalId, binding.sourceId, snapshot.record, snapshot.hash)
                    !snapshot.present && binding.providerRowId != null ->
                        ContactOutboundChange.Tombstone(binding.canonicalId, binding.sourceId, snapshot.hash)
                    else -> null
                }
            }
            if (changes.isEmpty()) return
            val result = NativeContactsOutboundSource(account.name, account.type, bridge).publish(changes)
            when (result) {
                org.anycal.android.contacts.ContactPublishResult.Success -> {
                    changes.forEach { change ->
                        val binding = store.get(account.name, account.type, ContactsContract.AUTHORITY, change.canonicalId)
                            ?: error("provider edit binding disappeared")
                        when (change) {
                            is ContactOutboundChange.Upsert -> store.upsert(binding.copy(
                                providerRowId = snapshots.getValue(change.canonicalId).rowId,
                                projectedHash = change.record.let { org.anycal.android.contacts.ContactsProjectionMapper(account.name, account.type).projectionHash(it) },
                                observedHash = change.observedHash,
                            ))
                            is ContactOutboundChange.Tombstone -> store.upsert(binding.copy(
                                providerRowId = null,
                                projectedHash = null,
                                observedHash = null,
                                tombstoneRevision = (binding.canonicalRevision?.toLongOrNull() ?: 0L).plus(1L).toString(),
                            ))
                        }
                    }
                }
                is org.anycal.android.contacts.ContactPublishResult.Retryable -> error(result.reason)
                is org.anycal.android.contacts.ContactPublishResult.Rejected -> error(result.reason)
            }
        }
    }

    private class Adapter(context: Context) : AbstractThreadedSyncAdapter(context, true, false) {
        override fun onPerformSync(
            account: Account,
            extras: Bundle,
            authority: String,
            provider: ContentProviderClient,
            syncResult: SyncResult,
        ) {
            if (authority != ContactsContract.AUTHORITY) {
                syncResult.stats.numIoExceptions++
                return
            }
            runCatching {
                performConfiguredContactsSync(context, account, configuredBridge(context, account))
            }.onFailure {
                // Transport, permission, and provider failures are surfaced to Android's
                // scheduler and never converted into provider writes or fake success.
                syncResult.stats.numIoExceptions++
            }
        }

        private fun configuredBridge(context: Context, account: Account): RustSyncBridge = with(context.getSharedPreferences("anycal_bridge", Context.MODE_PRIVATE)) {
            val tokenProvider = { AccountManager.get(context).peekAuthToken(account, "anycal") }
            RustSyncBridgeFactory.create(BridgeRuntimeConfig(
                endpoint = getString("endpoint", "") ?: "",
                credentialHandle = getString("credential_handle", ANYCAL_ACCOUNT_TOKEN_KEY) ?: ANYCAL_ACCOUNT_TOKEN_KEY,
                enabled = getBoolean("enabled", false),
            ), tokenProvider)
        }
    }
}
