package org.anycal.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat

/** Read-only capability facts. This class intentionally performs no provider writes. */
data class ProviderCapabilities(
    val apiLevel: Int,
    val contactsAuthority: Boolean,
    val calendarAuthority: Boolean,
    val canReadContacts: Boolean,
    val canWriteContacts: Boolean,
    val canReadCalendar: Boolean,
    val canWriteCalendar: Boolean,
    val tasksOrgAuthority: Boolean,
    val tasksOrgVersionCode: Long?,
) {
    val tasksOrgSupported: Boolean
        get() = tasksOrgAuthority && tasksOrgVersionCode == 151202L

    companion object {
        fun probe(context: Context): ProviderCapabilities {
            val packageManager = context.packageManager
            val tasksProvider = packageManager.resolveContentProvider("org.tasks.api", 0)
            val tasksVersion = tasksProvider?.packageName?.let { packageName ->
                runCatching {
                    val packageInfo = packageManager.getPackageInfo(packageName, 0)
                    if (Build.VERSION.SDK_INT >= 28) {
                        packageInfo.longVersionCode
                    } else {
                        @Suppress("DEPRECATION")
                        packageInfo.versionCode.toLong()
                    }
                }.getOrNull()
            }
            fun granted(permission: String) =
                ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

            return ProviderCapabilities(
                apiLevel = Build.VERSION.SDK_INT,
                contactsAuthority = packageManager.resolveContentProvider("com.android.contacts", 0) != null,
                calendarAuthority = packageManager.resolveContentProvider("com.android.calendar", 0) != null,
                canReadContacts = granted(Manifest.permission.READ_CONTACTS),
                canWriteContacts = granted(Manifest.permission.WRITE_CONTACTS),
                canReadCalendar = granted(Manifest.permission.READ_CALENDAR),
                canWriteCalendar = granted(Manifest.permission.WRITE_CALENDAR),
                tasksOrgAuthority = tasksProvider != null,
                tasksOrgVersionCode = tasksVersion,
            )
        }
    }
}
