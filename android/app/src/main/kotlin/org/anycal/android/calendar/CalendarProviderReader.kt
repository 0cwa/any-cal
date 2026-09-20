package org.anycal.android.calendar

import android.database.Cursor
import android.content.Context
import android.provider.CalendarContract
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** Read-only lookup of account-owned provider IDs. Calendar/event IDs are
 * operational bindings; _SYNC_ID remains the stable Any-Cal source identity. */
class ContentResolverCalendarReader(
    private val context: Context,
    private val accountName: String,
    private val accountType: String,
) {
    fun snapshots(sourceIds: Set<String>): List<CalendarProviderSnapshot> {
        require(accountName.isNotBlank() && accountType.isNotBlank()) { "calendar account scope is required" }
        return sourceIds.filter { it.isNotBlank() }.sorted().flatMap { sourceId ->
            findCalendar(sourceId)?.let { calendar ->
                listOf(
                    CalendarProviderSnapshot(
                        sourceId = sourceId,
                        projectionHash = null,
                        providerId = null,
                        calendarProviderId = calendar,
                        accountName = accountName,
                        accountType = accountType,
                    ),
                )
            } ?: findEvent(sourceId)?.let { event ->
                val calendarId = event.second
                val snapshots = mutableListOf(
                    CalendarProviderSnapshot(
                        sourceId = sourceId,
                        projectionHash = null,
                        providerId = event.first,
                        calendarProviderId = calendarId,
                        accountName = accountName,
                        accountType = accountType,
                    ),
                )
                findCalendarSource(calendarId)?.let { calendarSource ->
                    snapshots += CalendarProviderSnapshot(
                        sourceId = calendarSource,
                        projectionHash = null,
                        providerId = null,
                        calendarProviderId = calendarId,
                        accountName = accountName,
                        accountType = accountType,
                    )
                }
                snapshots
            } ?: emptyList()
        }.distinctBy { it.sourceId }
    }

    /** Read account-owned events for provider-to-Anytype reconciliation. The
     * query is bounded and only accepts rows carrying our stable _SYNC_ID;
     * an unavailable provider returns an error and never becomes an empty
     * batch that could authorize remote deletions. */
    fun ownedEventBatch(bindings: List<org.anycal.android.sync.ProjectionBinding>, limit: Int): Result<CalendarProviderBatch> =
        runCatching {
            require(limit > 0) { "calendar provider batch limit must be positive" }
            val calendars = ownedCalendars()
            if (calendars.isEmpty()) {
                val tombstones = bindings
                    .asSequence()
                    .filter { it.providerRowId != null && it.tombstoneRevision == null }
                    .map { binding ->
                        CalendarObservedRecord(
                            sourceId = binding.sourceId,
                            envelope = null,
                            projectionHash = null,
                            operationId = null,
                            deleted = true,
                        )
                    }
                    .toList()
                check(tombstones.size <= limit) { "calendar provider batch exceeds limit" }
                return@runCatching CalendarProviderBatch(tombstones, null)
            }
            val bindingBySource = bindings.associateBy { it.sourceId }
            val prefix = "android/$accountType/$accountName/"
            val ids = calendars.keys.sorted()
            val placeholders = ids.joinToString(",") { "?" }
            val arguments = ids.map(Long::toString).toTypedArray()
            val projection = arrayOf(
                CalendarContract.Events._ID,
                CalendarContract.Events._SYNC_ID,
                CalendarContract.Events.CALENDAR_ID,
                CalendarContract.Events.UID_2445,
                CalendarContract.Events.TITLE,
                CalendarContract.Events.DESCRIPTION,
                CalendarContract.Events.EVENT_LOCATION,
                CalendarContract.Events.DTSTART,
                CalendarContract.Events.DTEND,
                CalendarContract.Events.DURATION,
                CalendarContract.Events.ALL_DAY,
                CalendarContract.Events.EVENT_TIMEZONE,
                CalendarContract.Events.EVENT_END_TIMEZONE,
                CalendarContract.Events.STATUS,
                CalendarContract.Events.RRULE,
                CalendarContract.Events.RDATE,
                CalendarContract.Events.EXDATE,
            )
            val rows = mutableListOf<CalendarObservedRecord>()
            context.contentResolver.query(
                CalendarContract.Events.CONTENT_URI,
                projection,
                "${CalendarContract.Events.CALENDAR_ID} IN ($placeholders)",
                arguments,
                "${CalendarContract.Events._ID} ASC",
            )?.use { cursor ->
                while (cursor.moveToNext()) {
                    val sourceId = cursor.textOrNull(CalendarContract.Events._SYNC_ID) ?: continue
                    if (!sourceId.startsWith(prefix)) continue
                    val calendarId = cursor.getLong(cursor.getColumnIndexOrThrow(CalendarContract.Events.CALENDAR_ID))
                    val calendar = calendars[calendarId] ?: continue
                    val canonicalId = bindingBySource[sourceId]?.canonicalId
                        ?: sourceId.removePrefix(prefix).takeIf { it.isNotBlank() }
                        ?: continue
                    val envelope = cursor.toCalendarEnvelope(
                        canonicalId = canonicalId,
                        sourceId = sourceId,
                        calendar = calendar,
                        revision = bindingBySource[sourceId]?.canonicalRevision ?: "0",
                    )
                    rows += CalendarObservedRecord(
                        sourceId = sourceId,
                        envelope = envelope,
                        projectionHash = CalendarContractProjection.projectionHash(envelope),
                        operationId = null,
                    )
                }
            } ?: error("CalendarContract event query returned no cursor")

            val present = rows.mapTo(mutableSetOf()) { it.sourceId }
            bindings.asSequence()
                .filter { it.providerRowId != null && it.tombstoneRevision == null && it.sourceId !in present }
                .map { binding ->
                    CalendarObservedRecord(
                        sourceId = binding.sourceId,
                        envelope = null,
                        projectionHash = null,
                        operationId = null,
                        deleted = true,
                    )
                }
                .forEach(rows::add)
            check(rows.size <= limit) { "calendar provider batch exceeds limit" }
            CalendarProviderBatch(rows.sortedBy { it.sourceId }, null)
        }

    private fun findCalendar(sourceId: String): Long? =
        context.contentResolver.query(
            CalendarContract.Calendars.CONTENT_URI,
            arrayOf(CalendarContract.Calendars._ID),
            "${CalendarContract.Calendars.ACCOUNT_NAME}=? AND ${CalendarContract.Calendars.ACCOUNT_TYPE}=? AND ${CalendarContract.Calendars._SYNC_ID}=?",
            arrayOf(accountName, accountType, sourceId),
            null,
        )?.use { cursor -> if (cursor.moveToFirst()) cursor.getLong(0) else null }

    private fun findCalendarSource(calendarId: Long): String? =
        context.contentResolver.query(
            CalendarContract.Calendars.CONTENT_URI,
            arrayOf(CalendarContract.Calendars._SYNC_ID),
            "${CalendarContract.Calendars._ID}=? AND ${CalendarContract.Calendars.ACCOUNT_NAME}=? AND ${CalendarContract.Calendars.ACCOUNT_TYPE}=?",
            arrayOf(calendarId.toString(), accountName, accountType),
            null,
        )?.use { cursor -> if (cursor.moveToFirst()) cursor.getString(0) else null }

    private fun findEvent(sourceId: String): Pair<Long, Long>? =
        context.contentResolver.query(
            CalendarContract.Events.CONTENT_URI,
            arrayOf(CalendarContract.Events._ID, CalendarContract.Events.CALENDAR_ID),
            "${CalendarContract.Events._SYNC_ID}=?",
            arrayOf(sourceId),
            null,
        )?.use { cursor ->
            if (!cursor.moveToFirst()) return@use null
            val eventId = cursor.getLong(0)
            val calendarId = cursor.getLong(1)
            val owned = context.contentResolver.query(
                CalendarContract.Calendars.CONTENT_URI,
                arrayOf(CalendarContract.Calendars._ID),
                "${CalendarContract.Calendars._ID}=? AND ${CalendarContract.Calendars.ACCOUNT_NAME}=? AND ${CalendarContract.Calendars.ACCOUNT_TYPE}=?",
                arrayOf(calendarId.toString(), accountName, accountType),
                null,
            )?.use { calendars -> calendars.moveToFirst() } == true
            if (owned) eventId to calendarId else null
        }

    private data class OwnedCalendar(
        val id: Long,
        val sourceId: String,
        val name: String,
        val timezone: String?,
    )

    private fun ownedCalendars(): Map<Long, OwnedCalendar> {
        val result = linkedMapOf<Long, OwnedCalendar>()
        context.contentResolver.query(
            CalendarContract.Calendars.CONTENT_URI,
            arrayOf(
                CalendarContract.Calendars._ID,
                CalendarContract.Calendars._SYNC_ID,
                CalendarContract.Calendars.CALENDAR_DISPLAY_NAME,
                CalendarContract.Calendars.NAME,
                CalendarContract.Calendars.CALENDAR_TIME_ZONE,
            ),
            "${CalendarContract.Calendars.ACCOUNT_NAME}=? AND ${CalendarContract.Calendars.ACCOUNT_TYPE}=?",
            arrayOf(accountName, accountType),
            "${CalendarContract.Calendars._ID} ASC",
        )?.use { cursor ->
            while (cursor.moveToNext()) {
                val id = cursor.getLong(cursor.getColumnIndexOrThrow(CalendarContract.Calendars._ID))
                val sourceId = cursor.textOrNull(CalendarContract.Calendars._SYNC_ID) ?: continue
                val name = cursor.textOrNull(CalendarContract.Calendars.CALENDAR_DISPLAY_NAME)
                    ?: cursor.textOrNull(CalendarContract.Calendars.NAME)
                    ?: sourceId
                result[id] = OwnedCalendar(
                    id = id,
                    sourceId = sourceId,
                    name = name,
                    timezone = cursor.textOrNull(CalendarContract.Calendars.CALENDAR_TIME_ZONE),
                )
            }
        } ?: error("CalendarContract calendar query returned no cursor")
        return result
    }

    private fun Cursor.toCalendarEnvelope(
        canonicalId: String,
        sourceId: String,
        calendar: OwnedCalendar,
        revision: String,
    ): CalendarEnvelope {
        val timezone = textOrNull(CalendarContract.Events.EVENT_TIMEZONE)?.takeIf { it.isNotBlank() }
            ?: calendar.timezone?.takeIf { it.isNotBlank() }
            ?: "UTC"
        val allDay = getInt(getColumnIndexOrThrow(CalendarContract.Events.ALL_DAY)) != 0
        val startMillis = getLong(getColumnIndexOrThrow(CalendarContract.Events.DTSTART))
        val endIndex = getColumnIndexOrThrow(CalendarContract.Events.DTEND)
        val endMillis = if (isNull(endIndex)) null else getLong(endIndex)
        val sourceUid = textOrNull(CalendarContract.Events.UID_2445)
            ?.takeIf { it.isNotBlank() }
            ?: canonicalId
        return CalendarEnvelope(
            canonicalId = canonicalId,
            davUid = sourceUid,
            calendarCanonicalId = calendar.sourceId.removePrefix("android/$accountType/$accountName/"),
            calendarName = calendar.name,
            title = textOrNull(CalendarContract.Events.TITLE).orEmpty(),
            description = textOrNull(CalendarContract.Events.DESCRIPTION),
            location = textOrNull(CalendarContract.Events.EVENT_LOCATION),
            start = CalendarDateTime(formatDateTime(startMillis, timezone, allDay), timezone, allDay),
            end = endMillis?.let { CalendarDateTime(formatDateTime(it, timezone, allDay), timezone, allDay) },
            duration = textOrNull(CalendarContract.Events.DURATION),
            status = nullableInt(CalendarContract.Events.STATUS),
            recurrenceRule = textOrNull(CalendarContract.Events.RRULE),
            recurrenceDates = textOrNull(CalendarContract.Events.RDATE)
                ?.split(',')
                ?.filter { it.isNotBlank() }
                .orEmpty(),
            exceptionDates = textOrNull(CalendarContract.Events.EXDATE)
                ?.split(',')
                ?.filter { it.isNotBlank() }
                .orEmpty(),
            attendees = readAttendees(getLong(getColumnIndexOrThrow(CalendarContract.Events._ID))),
            reminders = readReminders(getLong(getColumnIndexOrThrow(CalendarContract.Events._ID))),
            revision = revision,
        )
    }

    private fun readAttendees(eventId: Long): List<CalendarAttendee> =
        context.contentResolver.query(
            CalendarContract.Attendees.CONTENT_URI,
            arrayOf(
                CalendarContract.Attendees.ATTENDEE_EMAIL,
                CalendarContract.Attendees.ATTENDEE_NAME,
                CalendarContract.Attendees.ATTENDEE_RELATIONSHIP,
                CalendarContract.Attendees.ATTENDEE_STATUS,
                CalendarContract.Attendees.ATTENDEE_TYPE,
            ),
            "${CalendarContract.Attendees.EVENT_ID}=?",
            arrayOf(eventId.toString()),
            null,
        )?.use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    val email = cursor.textOrNull(CalendarContract.Attendees.ATTENDEE_EMAIL)
                        ?: continue
                    add(
                        CalendarAttendee(
                            email = email,
                            name = cursor.textOrNull(CalendarContract.Attendees.ATTENDEE_NAME),
                            relationship = cursor.nullableInt(CalendarContract.Attendees.ATTENDEE_RELATIONSHIP)
                                ?: CalendarContract.Attendees.RELATIONSHIP_NONE,
                            status = cursor.nullableInt(CalendarContract.Attendees.ATTENDEE_STATUS)
                                ?: CalendarContract.Attendees.ATTENDEE_STATUS_NONE,
                            type = cursor.nullableInt(CalendarContract.Attendees.ATTENDEE_TYPE)
                                ?: CalendarContract.Attendees.TYPE_REQUIRED,
                        ),
                    )
                }
            }
        } ?: error("CalendarContract attendee query returned no cursor")

    private fun readReminders(eventId: Long): List<CalendarReminder> =
        context.contentResolver.query(
            CalendarContract.Reminders.CONTENT_URI,
            arrayOf(CalendarContract.Reminders.MINUTES, CalendarContract.Reminders.METHOD),
            "${CalendarContract.Reminders.EVENT_ID}=?",
            arrayOf(eventId.toString()),
            null,
        )?.use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    val minutes = cursor.nullableInt(CalendarContract.Reminders.MINUTES) ?: continue
                    add(
                        CalendarReminder(
                            minutesBefore = minutes,
                            method = cursor.nullableInt(CalendarContract.Reminders.METHOD)
                                ?: CalendarContract.Reminders.METHOD_ALERT,
                        ),
                    )
                }
            }
        } ?: error("CalendarContract reminder query returned no cursor")

    private fun formatDateTime(millis: Long, timezone: String, allDay: Boolean): String {
        val instant = Instant.ofEpochMilli(millis)
        return if (allDay) {
            instant.atZone(ZoneId.of(timezone)).toLocalDate().toString()
        } else {
            instant.toString()
        }
    }
}

private fun Cursor.textOrNull(column: String): String? {
    val index = getColumnIndexOrThrow(column)
    return if (isNull(index)) null else getString(index)
}

private fun Cursor.nullableInt(column: String): Int? {
    val index = getColumnIndexOrThrow(column)
    return if (isNull(index)) null else getInt(index)
}
