package org.anycal.android.calendar

import android.content.ContentValues
import android.provider.CalendarContract
import org.anycal.android.BridgeResource
import java.security.MessageDigest
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** Provider-neutral values supplied by the Rust/Anytype boundary. */
data class CalendarDateTime(
    val value: String,
    val timezone: String? = null,
    val allDay: Boolean = false,
) {
    fun epochMillis(): Long? = runCatching {
        if (allDay) {
            val date = runCatching { LocalDate.parse(value.take(10)) }.getOrElse {
                LocalDate.parse(value.take(8), DateTimeFormatter.ofPattern("yyyyMMdd"))
            }
            date
                .atStartOfDay(ZoneId.of(timezone ?: "UTC"))
                .toInstant()
                .toEpochMilli()
        } else {
            runCatching { Instant.parse(value).toEpochMilli() }
                .recoverCatching { OffsetDateTime.parse(value).toInstant().toEpochMilli() }
                .recoverCatching {
                    LocalDateTime.parse(value)
                        .atZone(ZoneId.of(timezone ?: "UTC"))
                        .toInstant()
                        .toEpochMilli()
                }
                .recoverCatching {
                    val compact = value.removeSuffix("Z")
                    LocalDateTime.parse(compact, DateTimeFormatter.ofPattern("yyyyMMdd'T'HHmmss"))
                        .atZone(ZoneId.of(timezone ?: "UTC"))
                        .toInstant()
                        .toEpochMilli()
                }
                .getOrThrow()
        }
    }.getOrNull()
}

data class CalendarAttendee(
    val email: String,
    val name: String? = null,
    val relationship: Int = CalendarContract.Attendees.RELATIONSHIP_NONE,
    val status: Int = CalendarContract.Attendees.ATTENDEE_STATUS_NONE,
    val type: Int = CalendarContract.Attendees.TYPE_REQUIRED,
)

data class CalendarReminder(
    val minutesBefore: Int,
    val method: Int = CalendarContract.Reminders.METHOD_ALERT,
)

data class OpaqueCalendarProperty(
    val name: String,
    val value: String,
    val parameters: Map<String, List<String>> = emptyMap(),
    val order: Int = 0,
)

data class CalendarEnvelope(
    val canonicalId: String,
    val davUid: String,
    val calendarCanonicalId: String,
    val calendarName: String,
    val title: String,
    val description: String? = null,
    val location: String? = null,
    val start: CalendarDateTime,
    val end: CalendarDateTime? = null,
    val duration: String? = null,
    val status: Int? = null,
    val transparency: Int? = null,
    val recurrenceRule: String? = null,
    val recurrenceDates: List<String> = emptyList(),
    val exceptionDates: List<String> = emptyList(),
    val attendees: List<CalendarAttendee> = emptyList(),
    val reminders: List<CalendarReminder> = emptyList(),
    val opaque: List<OpaqueCalendarProperty> = emptyList(),
    val revision: String,
    val deleted: Boolean = false,
)

data class CalendarProviderCapability(
    val authorityPresent: Boolean,
    val canRead: Boolean,
    val canWrite: Boolean,
    val reason: String? = null,
) {
    val readable: Boolean get() = authorityPresent && canRead
    val writable: Boolean get() = readable && canWrite
}

sealed interface CalendarProjectionResult {
    data class Planned(val plan: CalendarProjectionPlan) : CalendarProjectionResult
    data class Unsupported(val reason: String) : CalendarProjectionResult
}

data class CalendarProjectionPlan(
    val calendarSourceId: String,
    val eventSourceId: String,
    val calendarValues: ContentValues,
    val eventValues: ContentValues?,
    val attendees: List<ContentValues>,
    val reminders: List<ContentValues>,
    val opaque: List<OpaqueCalendarProperty>,
    val projectionHash: String,
    val tombstone: Boolean,
)

/**
 * Deterministic, side-effect-free CalendarContract projection planner.
 * Applying the returned values is deliberately a separate Android operation.
 */
object CalendarContractProjection {
    private const val SYNC_ID = "_sync_id"

    fun sourceId(accountType: String, accountName: String, canonicalId: String): String =
        "android/$accountType/$accountName/$canonicalId"

    /** Stable provider projection hash used to suppress echoes after a
     * successful Anytype-to-CalendarContract apply. */
    fun projectionHash(envelope: CalendarEnvelope): String = hash(envelope)

    fun plan(
        envelope: CalendarEnvelope,
        accountType: String,
        accountName: String,
        capability: CalendarProviderCapability,
    ): CalendarProjectionResult {
        if (!capability.writable) {
            return CalendarProjectionResult.Unsupported(
                capability.reason ?: "CalendarContract is not writable",
            )
        }

        val calendarSourceId = sourceId(accountType, accountName, envelope.calendarCanonicalId)
        val eventSourceId = sourceId(accountType, accountName, envelope.canonicalId)
        val calendarValues = ContentValues().apply {
            put(CalendarContract.Calendars.ACCOUNT_NAME, accountName)
            put(CalendarContract.Calendars.ACCOUNT_TYPE, accountType)
            put(CalendarContract.Calendars.NAME, envelope.calendarName)
            put(CalendarContract.Calendars.CALENDAR_DISPLAY_NAME, envelope.calendarName)
            put(CalendarContract.Calendars.OWNER_ACCOUNT, accountName)
            put(CalendarContract.Calendars.CALENDAR_COLOR, 0xff4f46e5.toInt())
            put(CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL, CalendarContract.Calendars.CAL_ACCESS_OWNER)
            put(CalendarContract.Calendars.CALENDAR_TIME_ZONE, envelope.start.timezone ?: "UTC")
            put(CalendarContract.Calendars.VISIBLE, 1)
            put(CalendarContract.Calendars.SYNC_EVENTS, 1)
            put(SYNC_ID, calendarSourceId)
        }

        val eventValues = if (envelope.deleted) {
            null
        } else {
            val startMillis = envelope.start.epochMillis()
                ?: return CalendarProjectionResult.Unsupported("Event start is not parseable")
            val endMillis = envelope.end?.epochMillis()
            if (envelope.end != null && endMillis == null) {
                return CalendarProjectionResult.Unsupported("Event end is not parseable")
            }
            ContentValues().apply {
                put(SYNC_ID, eventSourceId)
                put(CalendarContract.Events.UID_2445, envelope.davUid)
                put(CalendarContract.Events.TITLE, envelope.title)
                envelope.description?.let { put(CalendarContract.Events.DESCRIPTION, it) }
                envelope.location?.let { put(CalendarContract.Events.EVENT_LOCATION, it) }
                put(CalendarContract.Events.DTSTART, startMillis)
                endMillis?.let { put(CalendarContract.Events.DTEND, it) }
                envelope.duration?.let { put(CalendarContract.Events.DURATION, it) }
                put(CalendarContract.Events.ALL_DAY, if (envelope.start.allDay) 1 else 0)
                envelope.start.timezone?.let { put(CalendarContract.Events.EVENT_TIMEZONE, it) }
                envelope.end?.timezone?.let { put(CalendarContract.Events.EVENT_END_TIMEZONE, it) }
                envelope.status?.let { put(CalendarContract.Events.STATUS, it) }
                envelope.recurrenceRule?.let { put(CalendarContract.Events.RRULE, it) }
                if (envelope.recurrenceDates.isNotEmpty()) {
                    put(CalendarContract.Events.RDATE, envelope.recurrenceDates.joinToString(","))
                }
                if (envelope.exceptionDates.isNotEmpty()) {
                    put(CalendarContract.Events.EXDATE, envelope.exceptionDates.joinToString(","))
                }
            }
        }

        val attendees = envelope.attendees.map { attendee ->
            ContentValues().apply {
                put(CalendarContract.Attendees.ATTENDEE_EMAIL, attendee.email)
                attendee.name?.let { put(CalendarContract.Attendees.ATTENDEE_NAME, it) }
                put(CalendarContract.Attendees.ATTENDEE_RELATIONSHIP, attendee.relationship)
                put(CalendarContract.Attendees.ATTENDEE_STATUS, attendee.status)
                put(CalendarContract.Attendees.ATTENDEE_TYPE, attendee.type)
            }
        }
        val reminders = envelope.reminders.map { reminder ->
            ContentValues().apply {
                put(CalendarContract.Reminders.MINUTES, reminder.minutesBefore)
                put(CalendarContract.Reminders.METHOD, reminder.method)
            }
        }

        return CalendarProjectionResult.Planned(
            CalendarProjectionPlan(
                calendarSourceId = calendarSourceId,
                eventSourceId = eventSourceId,
                calendarValues = calendarValues,
                eventValues = eventValues,
                attendees = attendees,
                reminders = reminders,
                opaque = envelope.opaque,
                projectionHash = hash(envelope),
                tombstone = envelope.deleted,
            ),
        )
    }

    private fun hash(envelope: CalendarEnvelope): String {
        val canonical = listOf(
            envelope.canonicalId,
            envelope.davUid,
            envelope.calendarCanonicalId,
            envelope.title,
            envelope.description,
            envelope.location,
            envelope.start.epochMillis() ?: envelope.start.value,
            envelope.start.timezone,
            envelope.start.allDay,
            envelope.end?.epochMillis() ?: envelope.end?.value,
            envelope.end?.timezone,
            envelope.duration,
            envelope.status,
            envelope.transparency,
            envelope.recurrenceRule,
            envelope.recurrenceDates.sorted(),
            envelope.exceptionDates.sorted(),
            envelope.attendees.sortedBy { it.email },
            envelope.reminders.sortedBy { it.minutesBefore },
            envelope.deleted,
        ).joinToString("\u001f")
        return MessageDigest.getInstance("SHA-256")
            .digest(canonical.toByteArray())
            .joinToString("") { "%02x".format(it) }
    }
}

private fun BridgeResource.occurrence(name: String): org.anycal.android.BridgeOccurrence? =
    document.fields[name]?.firstOrNull()

private fun BridgeResource.occurrences(name: String): List<org.anycal.android.BridgeOccurrence> =
    document.fields[name].orEmpty()

/** Maps the canonical calendar projection view into the Android provider DTO.
 * Unknown properties remain in the Rust/DAV envelope and are not silently
 * converted into provider columns. */
fun BridgeResource.toCalendarEnvelope(): CalendarEnvelope {
    require(kind == "event") { "bridge resource is not an event" }
    val start = occurrence("DTSTART") ?: error("calendar event has no DTSTART")
    val startZone = start.params["TZID"]?.firstOrNull()
    val end = occurrence("DTEND")
    val attendees = occurrences("ATTENDEE").map { attendee ->
        CalendarAttendee(
            email = attendee.value.removePrefix("mailto:"),
            name = attendee.params["CN"]?.firstOrNull(),
            status = when (attendee.params["PARTSTAT"]?.firstOrNull()?.uppercase()) {
                "ACCEPTED" -> CalendarContract.Attendees.ATTENDEE_STATUS_ACCEPTED
                "DECLINED" -> CalendarContract.Attendees.ATTENDEE_STATUS_DECLINED
                "TENTATIVE" -> CalendarContract.Attendees.ATTENDEE_STATUS_TENTATIVE
                else -> CalendarContract.Attendees.ATTENDEE_STATUS_NONE
            },
            type = when (attendee.params["ROLE"]?.firstOrNull()?.uppercase()) {
                "OPT-PARTICIPANT" -> CalendarContract.Attendees.TYPE_OPTIONAL
                "NON-PARTICIPANT" -> CalendarContract.Attendees.TYPE_RESOURCE
                else -> CalendarContract.Attendees.TYPE_REQUIRED
            },
        )
    }
    val known = setOf(
        "UID", "SUMMARY", "DESCRIPTION", "LOCATION", "DTSTART", "DTEND", "DURATION",
        "STATUS", "TRANSP", "RRULE", "RDATE", "EXDATE", "ATTENDEE",
    )
    val opaque = document.fields.filterKeys { it !in known }.flatMap { (name, values) ->
        values.mapIndexed { index, value ->
            OpaqueCalendarProperty(name, value.value, value.params, index)
        }
    }
    return CalendarEnvelope(
        canonicalId = anytypeObjectId,
        davUid = occurrence("UID")?.value ?: davUid,
        calendarCanonicalId = collectionId,
        calendarName = collectionId,
        title = occurrence("SUMMARY")?.value.orEmpty(),
        description = occurrence("DESCRIPTION")?.value,
        location = occurrence("LOCATION")?.value,
        start = CalendarDateTime(start.value, startZone, start.value.length == 8),
        end = end?.let { CalendarDateTime(it.value, it.params["TZID"]?.firstOrNull(), it.value.length == 8) },
        duration = occurrence("DURATION")?.value,
        status = when (occurrence("STATUS")?.value?.uppercase()) {
            "TENTATIVE" -> CalendarContract.Events.STATUS_TENTATIVE
            "CONFIRMED" -> CalendarContract.Events.STATUS_CONFIRMED
            "CANCELLED" -> CalendarContract.Events.STATUS_CANCELED
            else -> null
        },
        transparency = null,
        recurrenceRule = occurrence("RRULE")?.value,
        recurrenceDates = occurrences("RDATE").flatMap { it.value.split(',') }.filter { it.isNotBlank() },
        exceptionDates = occurrences("EXDATE").flatMap { it.value.split(',') }.filter { it.isNotBlank() },
        attendees = attendees,
        opaque = opaque,
        revision = revision.toString(),
    )
}
