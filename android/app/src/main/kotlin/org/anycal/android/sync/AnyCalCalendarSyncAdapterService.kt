package org.anycal.android.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.Context
import android.content.SyncResult
import android.os.Bundle
import org.anycal.android.calendar.CalendarSyncRunner
import org.anycal.android.calendar.NativeCalendarBridgeSource
import org.anycal.android.BridgeRuntimeConfig
import org.anycal.android.RustSyncBridgeFactory
import org.anycal.android.account.ANYCAL_ACCOUNT_GENERATION_KEY
import org.anycal.android.account.ANYCAL_ACCOUNT_TOKEN_KEY
import android.provider.CalendarContract

/** Calendar account hook; the configured bridge supplies account-owned events. */
class AnyCalCalendarSyncAdapterService : android.app.Service() {
    private lateinit var adapter: Adapter

    override fun onCreate() {
        super.onCreate()
        adapter = Adapter(this)
    }

    override fun onBind(intent: android.content.Intent?) = adapter.syncAdapterBinder

    private class Adapter(context: Context) : AbstractThreadedSyncAdapter(context, true, false) {
        override fun onPerformSync(
            account: Account,
            extras: Bundle,
            authority: String,
            provider: ContentProviderClient,
            syncResult: SyncResult,
        ) {
            val bridge = configuredBridge(context, account)
            val store = ProjectionBindingStore(context)
            val generation = AccountManager.get(context).getUserData(account, ANYCAL_ACCOUNT_GENERATION_KEY)
            CalendarSyncRunner(
                context,
                NativeCalendarBridgeSource(context, bridge),
                onCheckpointCommitted = { checkpoint ->
                    if (generation != null) {
                        store.saveCheckpoint(account.name, account.type, CalendarContract.AUTHORITY, generation, checkpoint)
                    }
                },
                outboundBridge = bridge,
            ).run(account.name, account.type, authority)
                .onFailure { syncResult.stats.numIoExceptions++ }
        }

        private fun configuredBridge(context: Context, account: Account) = with(
            context.getSharedPreferences("anycal_bridge", Context.MODE_PRIVATE),
        ) {
            val tokenProvider = { AccountManager.get(context).peekAuthToken(account, "anycal") }
            RustSyncBridgeFactory.create(
                BridgeRuntimeConfig(
                    endpoint = getString("endpoint", "") ?: "",
                    credentialHandle = getString("credential_handle", ANYCAL_ACCOUNT_TOKEN_KEY) ?: ANYCAL_ACCOUNT_TOKEN_KEY,
                    enabled = getBoolean("enabled", false),
                ),
                tokenProvider,
            )
        }
    }
}
