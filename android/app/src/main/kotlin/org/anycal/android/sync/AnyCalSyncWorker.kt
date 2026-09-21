package org.anycal.android.sync

import android.content.Context
import android.accounts.Account
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import org.anycal.android.BridgeRuntimeConfig
import org.anycal.android.NativeRustBridge
import org.anycal.android.RustSyncBridgeFactory
import org.anycal.android.ProviderCapabilities
import org.anycal.android.tasks.TasksOrgSyncFactory

/** WorkManager hook for bounded bridge readiness checks; unavailable bridges fail closed. */
class AnyCalSyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        if (!NativeRustBridge.initializeVerifier(applicationContext)) return Result.failure()
        val prefs = applicationContext.getSharedPreferences("anycal_bridge", android.content.Context.MODE_PRIVATE)
        val bridge = RustSyncBridgeFactory.create(BridgeRuntimeConfig(
            prefs.getString("endpoint", "") ?: "",
            prefs.getString("credential_handle", "") ?: "",
            prefs.getBoolean("enabled", false),
        ))
        val result = bridge.syncOnce(ProviderCapabilities.probe(applicationContext))
        if (result !is org.anycal.android.SyncResult.Ready) return Result.failure()

        // The worker remains usable as a bridge-readiness worker. When the
        // scheduler supplies an account, run the real Tasks.org projection
        // from this background context; no UI or DAV intermediary is needed.
        val accountName = inputData.getString(INPUT_ACCOUNT_NAME)
        val accountType = inputData.getString(INPUT_ACCOUNT_TYPE)
        if (accountName != null && accountType != null) {
            val tasks = TasksOrgSyncFactory.run(
                applicationContext,
                Account(accountName, accountType),
                bridge,
            )
            if (tasks.isFailure) return Result.failure()
        }
        return Result.success()
    }

    companion object {
        const val INPUT_ACCOUNT_NAME = "account_name"
        const val INPUT_ACCOUNT_TYPE = "account_type"
    }
}
