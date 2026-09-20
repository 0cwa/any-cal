package org.anycal.android

import org.anycal.android.calendar.CalendarProjectionResult
import org.anycal.android.calendar.CalendarProviderCapability
import org.anycal.android.calendar.CalendarContractProjection
import org.anycal.android.calendar.toCalendarEnvelope
import org.anycal.android.contacts.ContactRecord
import org.anycal.android.contacts.LabeledValue
import org.anycal.android.contacts.toBridgeResource
import org.anycal.android.contacts.toContactRecord

/** Provider-free contract checks for full bridge payloads. */
object BridgePayloadChecks {
    @JvmStatic
    fun main(args: Array<String>) = runAll()

    fun runAll() {
        contactPayloadKeepsRepeatedValuesAndSeparateIdentities()
        calendarPayloadMapsKnownFieldsAndKeepsOpaqueFields()
        scopeAndTombstoneValidationFailsClosed()
    }

    private fun contactPayloadKeepsRepeatedValuesAndSeparateIdentities() {
        val contact = ContactRecord(
            anytypeObjectId = "object-1",
            displayName = "Synthetic Contact",
            givenName = "Synthetic",
            familyName = "Contact",
            phones = listOf(LabeledValue("+46-1", "cell"), LabeledValue("+46-2", "work")),
            emails = listOf(LabeledValue("one@example.invalid", "home"), LabeledValue("two@example.invalid", "work")),
            groups = setOf("friends", "crm"),
            canonicalRevision = "7",
        )
        val resource = contact.toBridgeResource()
        check(resource.resourceId != resource.anytypeObjectId)
        check(resource.davUid != resource.anytypeObjectId)
        check(resource.document.fields.getValue("TEL").size == 2)
        check(resource.document.fields.getValue("EMAIL").size == 2)
        check(resource.toContactRecord() == contact)
    }

    private fun calendarPayloadMapsKnownFieldsAndKeepsOpaqueFields() {
        val resource = BridgeResource(
            collectionId = "personal",
            resourceId = "event:1",
            kind = "event",
            anytypeObjectId = "object-event-1",
            davUid = "uid-1",
            document = BridgeDocument(
                mapOf(
                    "UID" to listOf(BridgeOccurrence("uid-1")),
                    "SUMMARY" to listOf(BridgeOccurrence("Meeting")),
                    "DTSTART" to listOf(BridgeOccurrence("20260102T100000Z")),
                    "DTEND" to listOf(BridgeOccurrence("20260102T110000Z")),
                    "X-CRM" to listOf(BridgeOccurrence("follow-up")),
                ),
            ),
            revision = 2,
        )
        val envelope = resource.toCalendarEnvelope()
        check(envelope.canonicalId == "object-event-1")
        check(envelope.davUid == "uid-1")
        check(envelope.title == "Meeting")
        check(envelope.opaque.single().name == "X-CRM")
        check(envelope.start.epochMillis() != null)
        check(CalendarContractProjection.plan(
            envelope,
            "org.anycal.account",
            "space-a",
            CalendarProviderCapability(true, true, true),
        ) is CalendarProjectionResult.Planned)
    }

    private fun scopeAndTombstoneValidationFailsClosed() {
        val resource = ContactRecord("object-1", "Contact").toBridgeResource()
        check(runCatching {
            BridgeRequest("space-a", "org.anycal.account", "com.android.contacts", null,
                resources = listOf("other"), tombstones = emptyList(), resourcePayloads = listOf(resource)).validate()
        }.isFailure)
        check(runCatching {
            BridgeRequest("space-a", "org.anycal.account", "com.android.contacts", null,
                resources = emptyList(), tombstones = listOf(BridgeTombstone("contact:1", "object-1", 0L))).validate()
        }.isFailure)
        check(runCatching {
            BridgeRequest("space-a", "org.anycal.account", "com.android.contacts", null,
                resources = emptyList(), tombstones = listOf(BridgeTombstone("contact:1", "object-1", -1L))).validate()
        }.isFailure)
    }
}
