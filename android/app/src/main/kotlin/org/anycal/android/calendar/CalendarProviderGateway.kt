package org.anycal.android.calendar

import android.content.ContentResolver

sealed interface CalendarGatewayResult {
    object Applied : CalendarGatewayResult
    data class Failed(val reason: String) : CalendarGatewayResult
}

/** Applies a callback plan in dependency order, retaining IDs only in memory until success. */
class CalendarContractGateway(
    private val store: CalendarContractStore,
    private val accountName: String,
    private val accountType: String,
    private val capability: CalendarProviderCapability,
) {
    fun apply(operations: List<CalendarProviderOperation>): CalendarGatewayResult {
        if (!capability.writable) return CalendarGatewayResult.Failed(capability.reason ?: "Calendar provider is not writable")
        val calendars = mutableMapOf<String, Long>()
        operations.forEach { operation ->
            when (operation) {
                is CalendarProviderOperation.NoOp -> Unit
                is CalendarProviderOperation.DeleteEvent -> operation.providerId?.let { store.deleteEvent(accountName, accountType, it) }
                is CalendarProviderOperation.DeleteCalendar -> operation.providerId?.let { store.deleteCalendar(accountName, accountType, it) }
                is CalendarProviderOperation.EnsureCalendar -> {
                    val id = operation.updateProviderId?.let {
                        if (store.updateCalendar(accountName, accountType, it, operation.values) <= 0) return CalendarGatewayResult.Failed("calendar update failed")
                        it
                    } ?: store.insertCalendar(accountName, accountType, operation.values)
                    if (id == null) return CalendarGatewayResult.Failed("calendar insert failed")
                    calendars[operation.sourceId] = id
                }
                is CalendarProviderOperation.EnsureEvent -> {
                    val calendarId = calendars[operation.calendarSourceId]
                        ?: store.findCalendarId(accountName, accountType, operation.calendarSourceId)
                        ?: return CalendarGatewayResult.Failed("calendar binding missing")
                    val eventId = operation.updateProviderId?.let {
                        if (store.replaceEvent(accountName, accountType, it, operation.values) <= 0) return CalendarGatewayResult.Failed("event update failed")
                        it
                    } ?: store.insertEvent(accountName, accountType, calendarId, operation.values)
                    if (eventId == null) return CalendarGatewayResult.Failed("event insert failed")
                    if (operation.updateProviderId != null) {
                        store.deleteAttendees(accountName, accountType, eventId)
                        store.deleteReminders(accountName, accountType, eventId)
                    }
                    if (store.insertAttendees(accountName, accountType, eventId, operation.attendees) != operation.attendees.size) return CalendarGatewayResult.Failed("attendee insert failed")
                    if (store.insertReminders(accountName, accountType, eventId, operation.reminders) != operation.reminders.size) return CalendarGatewayResult.Failed("reminder insert failed")
                }
            }
        }
        return CalendarGatewayResult.Applied
    }

    companion object {
        fun from(resolver: ContentResolver, accountName: String, accountType: String, capability: CalendarProviderCapability) =
            CalendarContractGateway(CalendarContractStore(resolver), accountName, accountType, capability)
    }
}
