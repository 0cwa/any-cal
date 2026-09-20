package org.anycal.android.calendar

/** Synthetic fixture used by host-side or Android instrumentation tests. */
object CalendarProjectionFixture {
    fun recurringEvent(): CalendarEnvelope = CalendarEnvelope(
        canonicalId = "anytype-event-synthetic-001",
        davUid = "event-synthetic-001@example.invalid",
        calendarCanonicalId = "calendar-synthetic-001",
        calendarName = "Any-Cal test calendar",
        title = "Synthetic review",
        description = "No personal data",
        location = "Test room",
        start = CalendarDateTime("2026-10-25T09:00:00", "Europe/Stockholm"),
        end = CalendarDateTime("2026-10-25T10:00:00", "Europe/Stockholm"),
        recurrenceRule = "FREQ=WEEKLY;COUNT=3",
        recurrenceDates = listOf("2026-11-15T09:00:00"),
        exceptionDates = listOf("2026-11-08T09:00:00"),
        attendees = listOf(CalendarAttendee("person@example.invalid")),
        reminders = listOf(CalendarReminder(15)),
        opaque = listOf(OpaqueCalendarProperty("X-SYNTHETIC-FIELD", "retain")),
        revision = "rev-1",
    )
}
