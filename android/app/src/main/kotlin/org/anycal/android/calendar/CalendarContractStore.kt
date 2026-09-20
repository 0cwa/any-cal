package org.anycal.android.calendar

import android.content.ContentResolver
import android.content.ContentUris
import android.content.ContentValues
import android.provider.CalendarContract

/**
 * Thin provider-writing seam. The Rust bridge supplies envelopes and owns
 * reconciliation; this class only applies an already-planned operation.
 */
class CalendarContractStore(
    private val resolver: ContentResolver,
) {
    fun findCalendarId(accountName: String, accountType: String, sourceId: String): Long? =
        findId(
            CalendarContract.Calendars.CONTENT_URI,
            "account_name = ? AND account_type = ? AND _sync_id = ?",
            arrayOf(accountName, accountType, sourceId),
        )

    fun findEventId(calendarId: Long, sourceId: String): Long? =
        findId(
            CalendarContract.Events.CONTENT_URI,
            "calendar_id = ? AND _sync_id = ?",
            arrayOf(calendarId.toString(), sourceId),
        )

    fun insertCalendar(accountName: String, accountType: String, values: ContentValues): Long? =
        resolver.insert(syncUri(CalendarContract.Calendars.CONTENT_URI, accountName, accountType), values)
            ?.lastPathSegment?.toLongOrNull()

    fun updateCalendar(accountName: String, accountType: String, calendarId: Long, values: ContentValues): Int =
        resolver.update(
            syncUri(ContentUris.withAppendedId(CalendarContract.Calendars.CONTENT_URI, calendarId), accountName, accountType),
            values,
            null,
            null,
        )

    fun deleteCalendar(accountName: String, accountType: String, calendarId: Long): Int =
        resolver.delete(
            syncUri(ContentUris.withAppendedId(CalendarContract.Calendars.CONTENT_URI, calendarId), accountName, accountType),
            null,
            null,
        )

    fun insertEvent(accountName: String, accountType: String, calendarId: Long, values: ContentValues): Long? {
        values.put(CalendarContract.Events.CALENDAR_ID, calendarId)
        return resolver.insert(syncUri(CalendarContract.Events.CONTENT_URI, accountName, accountType), values)
            ?.lastPathSegment?.toLongOrNull()
    }

    fun replaceEvent(accountName: String, accountType: String, eventId: Long, values: ContentValues): Int =
        resolver.update(
            syncUri(ContentUris.withAppendedId(CalendarContract.Events.CONTENT_URI, eventId), accountName, accountType),
            values,
            null,
            null,
        )

    fun deleteEvent(accountName: String, accountType: String, eventId: Long): Int =
        resolver.delete(
            syncUri(ContentUris.withAppendedId(CalendarContract.Events.CONTENT_URI, eventId), accountName, accountType),
            null,
            null,
        )

    private fun findId(uri: android.net.Uri, selection: String, args: Array<String>): Long? =
        resolver.query(uri, arrayOf(CalendarContract.Calendars._ID), selection, args, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getLong(0) else null
        }

    fun insertAttendees(accountName: String, accountType: String, eventId: Long, rows: List<ContentValues>): Int =
        rows.sumOf { row: ContentValues ->
            row.put(CalendarContract.Attendees.EVENT_ID, eventId)
            resolver.insert(syncUri(CalendarContract.Attendees.CONTENT_URI, accountName, accountType), row)
                ?.let { 1L } ?: 0L
        }.toInt()

    fun deleteAttendees(accountName: String, accountType: String, eventId: Long): Int =
        resolver.delete(
            syncUri(CalendarContract.Attendees.CONTENT_URI, accountName, accountType),
            "${CalendarContract.Attendees.EVENT_ID}=?",
            arrayOf(eventId.toString()),
        )

    fun insertReminders(accountName: String, accountType: String, eventId: Long, rows: List<ContentValues>): Int =
        rows.sumOf { row: ContentValues ->
            row.put(CalendarContract.Reminders.EVENT_ID, eventId)
            resolver.insert(syncUri(CalendarContract.Reminders.CONTENT_URI, accountName, accountType), row)
                ?.let { 1L } ?: 0L
        }.toInt()

    fun deleteReminders(accountName: String, accountType: String, eventId: Long): Int =
        resolver.delete(
            syncUri(CalendarContract.Reminders.CONTENT_URI, accountName, accountType),
            "${CalendarContract.Reminders.EVENT_ID}=?",
            arrayOf(eventId.toString()),
        )

    private fun syncUri(uri: android.net.Uri, accountName: String, accountType: String): android.net.Uri =
        uri.buildUpon().apply {
            syncAdapterQueryParameters(accountName, accountType).forEach { (key, value) ->
                appendQueryParameter(key, value)
            }
        }.build()

    companion object {
        internal fun syncAdapterQueryParameters(accountName: String, accountType: String): Map<String, String> = mapOf(
            CalendarContract.CALLER_IS_SYNCADAPTER to "true",
            CalendarContract.Calendars.ACCOUNT_NAME to accountName,
            CalendarContract.Calendars.ACCOUNT_TYPE to accountType,
        )
    }
}
