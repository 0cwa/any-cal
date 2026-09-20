package org.anycal.android.calendar

/** Provider-free acceptance checks for the deterministic callback boundary. */
object CalendarSyncCallbackAcceptanceTest {
    private val capability = CalendarProviderCapability(true, true, true)

    fun runAll() {
        rejectsIdentityAndPermissionFailures()
        plansCreateUpdateAndIdempotentNoOp()
        preservesOrderingAndPayload()
        plansTombstonesDeterministically()
    }

    private fun base(deleted: Boolean = false): CalendarEnvelope = CalendarEnvelope(
        canonicalId = "event-1", davUid = "uid-1", calendarCanonicalId = "calendar-1", calendarName = "Personal",
        title = "Meeting", start = CalendarDateTime("2026-01-02T10:00:00", "Europe/Stockholm"),
        end = CalendarDateTime("2026-01-02T11:00:00", "Europe/Stockholm"), recurrenceRule = "RRULE:FREQ=WEEKLY;COUNT=2",
        recurrenceDates = listOf("20260109T090000Z"), exceptionDates = listOf("20260116T090000Z"),
        attendees = listOf(CalendarAttendee("a@example.test")), reminders = listOf(CalendarReminder(15)),
        opaque = listOf(OpaqueCalendarProperty("X-TEST", "v")), revision = "r1", deleted = deleted,
    )

    private fun request(envelope: CalendarEnvelope, existing: List<CalendarProviderSnapshot> = emptyList()) = CalendarBridgeRequest(
        schemaVersion = 1, accountName = "user", accountType = "org.anycal", authority = "com.android.calendar",
        capability = capability, records = listOf(CalendarBridgeRecord(envelope, CalendarContractProjection.sourceId("org.anycal", "user", envelope.canonicalId))), existing = existing,
    )

    private fun rejectsIdentityAndPermissionFailures() {
        check(CalendarSyncCallback.validate(request(base()).copy(schemaVersion = 99)).isFailure)
        check(CalendarSyncCallback.validate(request(base()).copy(authority = "wrong")).isFailure)
        check(CalendarSyncCallback.validate(request(base()).copy(capability = CalendarProviderCapability(false, false, false, "absent"))).isFailure)
        val wrong = request(base()).copy(records = listOf(CalendarBridgeRecord(base(), "foreign/source")))
        check(CalendarSyncCallback.validate(wrong).isFailure)
    }

    private fun plansCreateUpdateAndIdempotentNoOp() {
        val first = CalendarSyncCallback.validate(request(base())).getOrThrow()
        val second = CalendarSyncCallback.validate(request(base()).copy(records = request(base()).records.reversed())).getOrThrow()
        check(first == second)
        check(CalendarSyncCallback.validate(request(base().copy(title = "Changed"))).isSuccess)
    }

    private fun preservesOrderingAndPayload() {
        val envelope = base()
        check(envelope.start.timezone == "Europe/Stockholm" && !envelope.start.allDay)
        check(envelope.recurrenceRule == "RRULE:FREQ=WEEKLY;COUNT=2")
        check(envelope.attendees.size == 1 && envelope.reminders.single().minutesBefore == 15)
        check(CalendarSyncCallback.validate(request(envelope)).getOrThrow().orderedSourceIds.single().endsWith("event-1"))
    }

    private fun plansTombstonesDeterministically() {
        val envelope = base(true)
        val result = CalendarSyncCallback.validate(request(envelope)).getOrThrow()
        check(result.tombstoneSourceIds == result.orderedSourceIds)
    }
}
