package org.anycal.android.tasks

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Disposable, debug-only provider gate. The receiver deliberately lives in
 * the Any-Cal process so ContentResolver calls run with the app UID and its
 * declared Tasks.org permissions. It never contacts Anytype or logs task
 * content. The host test drives PREPARE, restarts Tasks.org, then drives
 * RESTART_CHECK and FINISH.
 */
class TasksOrgProbeReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        val stage = intent.getStringExtra(EXTRA_STAGE)?.lowercase() ?: STAGE_PREPARE
        Thread {
            try {
                runStage(context.applicationContext, stage)
                pending.setResultCode(RESULT_OK)
            } catch (error: Throwable) {
                Log.e(TAG, "stage=$stage result=fail error=${error::class.java.simpleName}")
                pending.setResultCode(RESULT_FAILED)
            } finally {
                pending.finish()
            }
        }.start()
    }

    private fun runStage(context: Context, stage: String) {
        val adapter = TasksOrgAdapter(context)
        val capability = (adapter.probe() as? TasksOrgProbeResult.Supported)?.capability
            ?: error("Tasks.org capability unavailable")
        val gateway = ContentResolverTasksOrgGateway(context.contentResolver, adapter)
        val schema = gateway.schema()
        Log.i(
            TAG,
            "stage=$stage capability=ready version=${capability.versionCode} " +
                "task_columns=${schema.taskColumns.size} list_columns=${schema.listColumns.size}",
        )

        when (stage) {
            STAGE_PREPARE -> prepare(context, gateway)
            STAGE_RESTART_CHECK -> restartCheck(context, gateway)
            STAGE_FINISH -> finish(context, gateway)
            else -> error("unknown probe stage")
        }
    }

    private fun prepare(context: Context, gateway: TasksOrgProviderGateway) {
        val state = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        check(!state.contains(KEY_PROVIDER_ID)) { "probe state already exists" }
        val marker = "anycal-tasks-probe-${UUID.randomUUID()}"
        val insertedId = gateway.insert(TasksOrgTask(canonicalId = marker, title = marker))
        var keepRow = false
        try {
            val inserted = gateway.read(insertedId) ?: error("inserted task cannot be read")
            check(inserted.title == marker) { "inserted task title mismatch" }

            val observerLatch = CountDownLatch(1)
            val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
                override fun onChange(selfChange: Boolean) {
                    observerLatch.countDown()
                }
            }
            gateway.registerObserver(observer)
            try {
                val updatedTitle = "$marker-updated"
                check(gateway.update(insertedId, inserted.toTask(marker).copy(title = updatedTitle))) {
                    "updated task was not accepted"
                }
                val updated = gateway.read(insertedId) ?: error("updated task cannot be read")
                check(updated.title == updatedTitle) { "updated task title mismatch" }
                check(observerLatch.await(10, TimeUnit.SECONDS)) {
                    "Tasks.org did not notify the task collection observer"
                }
                state.edit()
                    .putLong(KEY_PROVIDER_ID, insertedId)
                    .putString(KEY_MARKER, marker)
                    .apply()
                keepRow = true
                Log.i(TAG, "stage=prepare result=pass crud=insert_update_read observer=pass")
            } finally {
                gateway.unregisterObserver(observer)
            }
        } finally {
            if (!keepRow) {
                runCatching { gateway.read(insertedId)?.let { gateway.deleteIfOwned(insertedId, it.hash()) } }
            }
        }
    }

    private fun restartCheck(context: Context, gateway: TasksOrgProviderGateway) {
        val state = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val providerId = state.getLong(KEY_PROVIDER_ID, 0L)
        val marker = state.getString(KEY_MARKER, null) ?: error("probe state is absent")
        check(providerId > 0L) { "probe provider ID is absent" }
        val row = gateway.read(providerId) ?: error("task disappeared after provider restart")
        check(row.title == "$marker-updated") { "task changed after provider restart" }
        Log.i(TAG, "stage=restart_check result=pass read=stable")
    }

    private fun finish(context: Context, gateway: TasksOrgProviderGateway) {
        val state = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val providerId = state.getLong(KEY_PROVIDER_ID, 0L)
        check(providerId > 0L) { "probe provider ID is absent" }
        val row = gateway.read(providerId) ?: error("task disappeared before delete")
        val observerLatch = CountDownLatch(1)
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                observerLatch.countDown()
            }
        }
        gateway.registerObserver(observer)
        try {
            check(gateway.deleteIfOwned(providerId, row.hash())) { "owned task delete was rejected" }
            check(gateway.read(providerId) == null) { "deleted task remains readable" }
            check(observerLatch.await(10, TimeUnit.SECONDS)) {
                "Tasks.org did not notify the task collection observer after delete"
            }
            state.edit().clear().apply()
            Log.i(TAG, "stage=finish result=pass crud=delete_read observer=pass")
        } finally {
            gateway.unregisterObserver(observer)
        }
    }

    private companion object {
        const val TAG = "AnyCalTasksProbe"
        const val EXTRA_STAGE = "stage"
        const val STAGE_PREPARE = "prepare"
        const val STAGE_RESTART_CHECK = "restart_check"
        const val STAGE_FINISH = "finish"
        const val PREFS = "tasks-org-debug-probe"
        const val KEY_PROVIDER_ID = "provider_id"
        const val KEY_MARKER = "marker"
        const val RESULT_OK = 0
        const val RESULT_FAILED = 1
    }
}
