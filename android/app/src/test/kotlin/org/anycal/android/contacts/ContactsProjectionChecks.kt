package org.anycal.android.contacts

/** Dependency-free deterministic acceptance checks for the provider-free mapper. */
object ContactsProjectionChecks {
    @JvmStatic
    fun main(args: Array<String>) {
        stableSourceId()
        labeledValuesAreRepeatedAndOrdered()
        groupsAreOrdered()
        hashIgnoresProviderRowOrder()
        tombstonesAreExplicit()
        identityMismatchIsRejected()
        capabilitiesFailClosed()
    }

    private val mapper = ContactsProjectionMapper("space-a", "invalid.example.anycal")

    private fun contact(
        phones: List<LabeledValue> = listOf(LabeledValue("+46-2", "work"), LabeledValue("+46-1", "cell")),
        emails: List<LabeledValue> = listOf(LabeledValue("b@example.invalid", "home"), LabeledValue("a@example.invalid", "work")),
        groups: Set<String> = setOf("zeta", "alpha"),
    ) = ContactRecord(
        anytypeObjectId = "object-1",
        displayName = "Synthetic Contact",
        givenName = "Synthetic",
        familyName = "Contact",
        phones = phones,
        emails = emails,
        groups = groups,
        canonicalRevision = "r1",
    )

    private fun stableSourceId() {
        check(mapper.sourceId("object-1") == "android/invalid.example.anycal/space-a/object-1")
    }

    private fun labeledValuesAreRepeatedAndOrdered() {
        val rows = mapper.plan(contact()).filterIsInstance<ContactProjectionOperation.ReplaceData>().single().rows
        check(rows.count { it.mimeType.endsWith("phone_v2") } == 2)
        check(rows.count { it.mimeType.endsWith("email_v2") } == 2)
        check(rows.filter { it.mimeType.endsWith("phone_v2") }.map { it.label } == listOf("cell", "work"))
        check(rows.filter { it.mimeType.endsWith("email_v2") }.map { it.label } == listOf("home", "work"))
    }

    private fun groupsAreOrdered() {
        val operations = mapper.plan(contact())
        val groups = operations.filterIsInstance<ContactProjectionOperation.EnsureGroups>().single().names
        val memberships = operations.filterIsInstance<ContactProjectionOperation.ReplaceMembership>().single().groupNames
        check(groups == listOf("alpha", "zeta"))
        check(memberships == groups)
    }

    private fun hashIgnoresProviderRowOrder() {
        val first = contact()
        val reordered = contact(phones = first.phones.reversed(), emails = first.emails.reversed(), groups = first.groups.reversed().toSet())
        check(mapper.projectionHash(first) == mapper.projectionHash(reordered))
    }

    private fun tombstonesAreExplicit() {
        val deleted = contact().copy(deleted = true, canonicalRevision = "r2")
        val operation = mapper.plan(deleted).single() as ContactProjectionOperation.Tombstone
        check(operation.sourceId == mapper.sourceId("object-1"))
        check(operation.canonicalRevision == "r2")
    }

    private fun identityMismatchIsRejected() {
        val previous = ContactProjectionState("android/invalid.example.anycal/space-a/other")
        check(runCatching { mapper.plan(contact(), previous) }.isFailure)
    }

    private fun capabilitiesFailClosed() {
        check(!ContactsCapabilities(false, true, true).writable)
        check(!ContactsCapabilities(true, false, true).writable)
        check(!ContactsCapabilities(true, true, false).writable)
        check(ContactsCapabilities(true, true, true).writable)
    }
}
