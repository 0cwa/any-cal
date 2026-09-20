package org.anycal.android.calendar

/** Deterministic fake-provider checks for Calendar outbound reconciliation. */
object CalendarBidirectionalReconcilerAcceptanceTest {
    fun runAll() {
        suppressesOwnEchoes()
        emitsLocalEditAndTombstone()
        rejectsConflictGenerationAndOversizedBatch()
        recoversCheckpointDeterministically()
    }

    private fun envelope(title: String = "Meeting") = CalendarEnvelope(
        canonicalId = "event-1", davUid = "uid-1", calendarCanonicalId = "calendar-1", calendarName = "Personal",
        title = title, start = CalendarDateTime("2026-01-02T10:00:00", "Europe/Stockholm"),
        end = CalendarDateTime("2026-01-02T11:00:00", "Europe/Stockholm"), recurrenceRule = "RRULE:FREQ=WEEKLY;COUNT=2",
        attendees = listOf(CalendarAttendee("person@example.test")), reminders = listOf(CalendarReminder(15)), revision = "r1",
    )

    private fun row(envelope: CalendarEnvelope = envelope(), hash: String? = "h", op: String? = "op") =
        CalendarObservedRecord("event-1", envelope, hash, op)

    private fun reconciler() = CalendarBidirectionalReconciler(batchSize = 2)

    private fun suppressesOwnEchoes() {
        val result = reconciler().reconcile(listOf(row()), listOf(CalendarBinding("event-1", "h", "op", 1)), CalendarReconcileCheckpoint("a", 1), "b", 1)
        check((result as CalendarReconcileResult.Applied).changes.isEmpty())
    }

    private fun emitsLocalEditAndTombstone() {
        val edit = reconciler().reconcile(listOf(row(envelope("Edited"), "changed", null)), emptyList(), CalendarReconcileCheckpoint(null, 1), "1", 1)
        check((edit as CalendarReconcileResult.Applied).changes.single() is CalendarOutboundChange.Upsert)
        val deleted = reconciler().reconcile(listOf(CalendarObservedRecord("event-1", null, null, null, true)), emptyList(), CalendarReconcileCheckpoint(null, 1), "2", 1)
        check((deleted as CalendarReconcileResult.Applied).changes.single() is CalendarOutboundChange.Tombstone)
    }

    private fun rejectsConflictGenerationAndOversizedBatch() {
        check(reconciler().reconcile(emptyList(), emptyList(), CalendarReconcileCheckpoint(null, 1), null, 2) is CalendarReconcileResult.Rejected)
        check(CalendarBidirectionalReconciler(1).reconcile(listOf(row(), row()), emptyList(), CalendarReconcileCheckpoint(null, 1), null, 1) is CalendarReconcileResult.Rejected)
    }

    private fun recoversCheckpointDeterministically() {
        val a = reconciler().reconcile(listOf(row()), emptyList(), CalendarReconcileCheckpoint(null, 1), "next", 1)
        val b = reconciler().reconcile(listOf(row()), emptyList(), CalendarReconcileCheckpoint(null, 1), "next", 1)
        check(a == b)
        check((a as CalendarReconcileResult.Applied).checkpoint.token == "next")
    }
}
