package org.anycal.android.calendar

import android.provider.CalendarContract
import java.time.Instant
import java.time.LocalDateTime
import java.time.ZoneId

/**
 * Dependency-free deterministic checks for the provider-free projection seam.
 * Run with a JVM/Android test harness by invoking [runAll]; no provider writes
 * or Android account state are required.
 */
object CalendarProjectionAcceptanceTest {
    private const val ACCOUNT_TYPE = "invalid.example.anycal"
    private const val ACCOUNT_NAME = "any-cal-test-space"

    @JvmStatic
    fun main(args: Array<String>) = runAll()

    fun runAll() {
        stableIdentityAndHash()
        dateAndProjectionFields()
        tombstoneAndFailClosedCapability()
    }

    private fun stableIdentityAndHash() {
        val fixture = CalendarProjectionFixture.recurringEvent()
        val capability = writableCapability()
        val first = planned(fixture, capability)
        val reordered = fixture.copy(
            recurrenceDates = fixture.recurrenceDates.reversed(),
            exceptionDates = fixture.exceptionDates.reversed(),
            attendees = fixture.attendees.reversed(),
            reminders = fixture.reminders.reversed(),
            opaque = fixture.opaque.reversed(),
        )
        val second = planned(reordered, capability)

        check(first.calendarSourceId == CalendarContractProjection.sourceId(
            ACCOUNT_TYPE,
            ACCOUNT_NAME,
            fixture.calendarCanonicalId,
        ))
        check(first.eventSourceId == CalendarContractProjection.sourceId(
            ACCOUNT_TYPE,
            ACCOUNT_NAME,
            fixture.canonicalId,
        ))
        check(first.projectionHash == second.projectionHash) {
            "reordering repeated values changed the projection hash"
        }
    }

    private fun dateAndProjectionFields() {
        val fixture = CalendarProjectionFixture.recurringEvent()
        val stockholmExpected = LocalDateTime.parse("2026-10-25T09:00:00")
            .atZone(ZoneId.of("Europe/Stockholm"))
            .toInstant()
            .toEpochMilli()
        check(fixture.start.epochMillis() == stockholmExpected)
        val plan = planned(fixture, writableCapability())
        val event = checkNotNull(plan.eventValues)

        check(event.getAsString(CalendarContract.Events.UID_2445) == fixture.davUid)
        check(event.getAsLong(CalendarContract.Events.DTSTART) == fixture.start.epochMillis())
        check(event.getAsLong(CalendarContract.Events.DTEND) == fixture.end?.epochMillis())
        check(event.getAsString(CalendarContract.Events.EVENT_TIMEZONE) == "Europe/Stockholm")
        check(event.getAsString(CalendarContract.Events.RRULE) == "FREQ=WEEKLY;COUNT=3")
        check(event.getAsString(CalendarContract.Events.RDATE) == "2026-11-15T09:00:00")
        check(event.getAsString(CalendarContract.Events.EXDATE) == "2026-11-08T09:00:00")
        check(plan.attendees.single().getAsString(CalendarContract.Attendees.ATTENDEE_EMAIL) == "person@example.invalid")
        check(plan.reminders.single().getAsInteger(CalendarContract.Reminders.MINUTES) == 15)
        check(plan.opaque.single().name == "X-SYNTHETIC-FIELD")

        val allDay = fixture.copy(start = CalendarDateTime("2026-10-25", "Europe/Stockholm", allDay = true))
        check(allDay.start.epochMillis() == Instant.parse("2026-10-24T22:00:00Z").toEpochMilli())
    }

    private fun tombstoneAndFailClosedCapability() {
        val fixture = CalendarProjectionFixture.recurringEvent()
        val tombstone = planned(fixture.copy(deleted = true), writableCapability())
        check(tombstone.tombstone)
        check(tombstone.eventValues == null)

        val denied = CalendarContractProjection.plan(
            fixture,
            ACCOUNT_TYPE,
            ACCOUNT_NAME,
            CalendarProviderCapability(
                authorityPresent = true,
                canRead = true,
                canWrite = false,
                reason = "WRITE_CALENDAR permission is not granted",
            ),
        )
        check(denied is CalendarProjectionResult.Unsupported)

        val absent = CalendarContractProjection.plan(
            fixture,
            ACCOUNT_TYPE,
            ACCOUNT_NAME,
            CalendarProviderCapability(
                authorityPresent = false,
                canRead = false,
                canWrite = false,
                reason = "Calendar provider authority is absent",
            ),
        )
        check(absent is CalendarProjectionResult.Unsupported)
    }

    private fun writableCapability() = CalendarProviderCapability(
        authorityPresent = true,
        canRead = true,
        canWrite = true,
    )

    private fun planned(
        fixture: CalendarEnvelope,
        capability: CalendarProviderCapability,
    ): CalendarProjectionPlan = when (
        val result = CalendarContractProjection.plan(fixture, ACCOUNT_TYPE, ACCOUNT_NAME, capability)
    ) {
        is CalendarProjectionResult.Planned -> result.plan
        is CalendarProjectionResult.Unsupported -> error(result.reason)
    }
}
