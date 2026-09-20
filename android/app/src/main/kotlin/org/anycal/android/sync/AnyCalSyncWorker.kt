package org.anycal.android.sync

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import org.anycal.android.BridgeRuntimeConfig
import org.anycal.android.RustSyncBridgeFactory
import org.anycal.android.ProviderCapabilities

/** WorkManager hook for bounded bridge readiness checks; unavailable bridges fail closed. */
class AnyCalSyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val prefs = applicationContext.getSharedPreferences("anycal_bridge", android.content.Context.MODE_PRIVATE)
        val result = RustSyncBridgeFactory.create(BridgeRuntimeConfig(
            prefs.getString("endpoint", "") ?: "",
            prefs.getString("credential_handle", "") ?: "",
            prefs.getBoolean("enabled", false),
        )).syncOnce(ProviderCapabilities.probe(applicationContext))
        return if (result is org.anycal.android.SyncResult.Ready) Result.success() else Result.failure()
    }
}
