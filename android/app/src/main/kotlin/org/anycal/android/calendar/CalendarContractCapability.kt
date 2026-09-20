package org.anycal.android.calendar

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.provider.CalendarContract
import androidx.core.content.ContextCompat

/** Read-only capability probe. It never creates accounts, calendars, or events. */
object CalendarContractCapability {
    fun probe(context: Context): CalendarProviderCapability {
        val authorityPresent = context.packageManager.resolveContentProvider(
            CalendarContract.AUTHORITY,
            0,
        ) != null
        val canRead = granted(context, Manifest.permission.READ_CALENDAR)
        val canWrite = granted(context, Manifest.permission.WRITE_CALENDAR)
        val reason = when {
            !authorityPresent -> "Calendar provider authority is absent"
            !canRead -> "READ_CALENDAR permission is not granted"
            !canWrite -> "WRITE_CALENDAR permission is not granted"
            else -> null
        }
        return CalendarProviderCapability(authorityPresent, canRead, canWrite, reason)
    }

    private fun granted(context: Context, permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
}
