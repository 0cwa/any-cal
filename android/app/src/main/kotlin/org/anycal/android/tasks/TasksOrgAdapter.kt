package org.anycal.android.tasks

import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.content.ContextCompat

data class TasksOrgVersionPolicy(
    val packageName: String = "org.tasks",
    /** Tasks.org 15.12 introduced the documented v0 API (versionCode 151202).
     * Newer versions must be added after their contract is revalidated. */
    val supportedVersionCodes: Set<Long> = setOf(151202L),
)

data class TasksOrgCapability(
    val packageName: String,
    val versionCode: Long,
    val authority: String,
    val apiVersion: String,
    val canRead: Boolean,
    val canWrite: Boolean,
) {
    val usable: Boolean get() = canRead && canWrite
}

sealed interface TasksOrgProbeResult {
    data class Supported(val capability: TasksOrgCapability) : TasksOrgProbeResult
    data class Unsupported(val reason: String) : TasksOrgProbeResult
}

/** Runtime-gated seam for the unstable Tasks.org v0 provider. It performs no CRUD. */
class TasksOrgAdapter(
    private val context: Context,
    private val versionPolicy: TasksOrgVersionPolicy = TasksOrgVersionPolicy(),
) {
    fun probe(): TasksOrgProbeResult {
        val provider = context.packageManager.resolveContentProvider(AUTHORITY, 0)
            ?: return TasksOrgProbeResult.Unsupported("provider authority is absent")
        if (provider.packageName != versionPolicy.packageName) {
            return TasksOrgProbeResult.Unsupported("provider package is not allow-listed")
        }
        val versionCode = packageVersionCode(provider.packageName)
            ?: return TasksOrgProbeResult.Unsupported("provider package version is unavailable")
        if (versionCode !in versionPolicy.supportedVersionCodes) {
            return TasksOrgProbeResult.Unsupported("provider version is not validated")
        }
        val capability = TasksOrgCapability(
            packageName = provider.packageName,
            versionCode = versionCode,
            authority = AUTHORITY,
            apiVersion = API_VERSION,
            canRead = granted(READ_PERMISSION),
            canWrite = granted(WRITE_PERMISSION),
        )
        if (!capability.usable) {
            return TasksOrgProbeResult.Unsupported("Tasks.org task permissions are missing or revoked")
        }
        return TasksOrgProbeResult.Supported(capability)
    }

    /** Capability probe for a background-safe caller. Provider schema validation
     * is deliberately separate because it performs a Binder read. */
    fun capability(): TasksOrgCapability? = when (val result = probe()) {
        is TasksOrgProbeResult.Supported -> result.capability
        is TasksOrgProbeResult.Unsupported -> null
    }

    fun baseUri(): Uri = BASE_URI

    private fun granted(permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    private fun packageVersionCode(packageName: String): Long? = runCatching {
        val info = context.packageManager.getPackageInfo(packageName, 0)
        if (Build.VERSION.SDK_INT >= 28) {
            info.longVersionCode
        } else {
            @Suppress("DEPRECATION")
            info.versionCode.toLong()
        }
    }.getOrNull()

    companion object {
        const val AUTHORITY = "org.tasks.api"
        const val API_VERSION = "v0"
        const val READ_PERMISSION = "org.tasks.permission.READ_TASKS"
        const val WRITE_PERMISSION = "org.tasks.permission.WRITE_TASKS"
        val BASE_URI: Uri = Uri.parse("content://$AUTHORITY/$API_VERSION")
    }
}
