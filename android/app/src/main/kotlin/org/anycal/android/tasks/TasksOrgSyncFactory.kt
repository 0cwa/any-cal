package org.anycal.android.tasks

import android.accounts.Account
import android.accounts.AccountManager
import android.content.Context
import org.anycal.android.RustSyncBridge
import org.anycal.android.account.ANYCAL_ACCOUNT_GENERATION_KEY
import org.anycal.android.sync.ProjectionBindingStore

/** Application integration seam. The caller schedules this from a background
 * worker or sync callback; provider work is never performed on the UI thread. */
object TasksOrgSyncFactory {
    fun run(
        context: Context,
        account: Account,
        bridge: RustSyncBridge,
    ): Result<TasksOrgSyncReport> {
        val generation = AccountManager.get(context).getUserData(account, ANYCAL_ACCOUNT_GENERATION_KEY)
            ?: return Result.failure(IllegalStateException("Tasks.org account generation is unavailable"))
        val adapter = TasksOrgAdapter(context)
        if (adapter.probe() !is TasksOrgProbeResult.Supported) {
            return Result.failure(UnsupportedOperationException("Tasks.org provider is absent, unvalidated, or permission-gated"))
        }
        return TasksOrgSyncRunner(
            account = account,
            store = ProjectionBindingStore(context),
            gateway = ContentResolverTasksOrgGateway(context.contentResolver, adapter),
            bridge = bridge,
            generation = generation,
        ).run()
    }
}
