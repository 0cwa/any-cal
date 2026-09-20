package org.anycal.android.contacts

import android.provider.ContactsContract
import org.anycal.android.BridgeDocument
import org.anycal.android.BridgeOccurrence
import org.anycal.android.BridgeResource
import java.security.MessageDigest

/** Canonical contact values supplied by the Rust sync bridge. */
data class ContactRecord(
    val anytypeObjectId: String,
    val displayName: String,
    val givenName: String = "",
    val familyName: String = "",
    val phones: List<LabeledValue> = emptyList(),
    val emails: List<LabeledValue> = emptyList(),
    val notes: List<String> = emptyList(),
    val groups: Set<String> = emptySet(),
    val deleted: Boolean = false,
    val canonicalRevision: String = "",
)

data class LabeledValue(val value: String, val label: String = "")

private fun List<BridgeOccurrence>.values(): List<String> = map { it.value }

private fun BridgeOccurrence.label(): String = params["TYPE"]?.firstOrNull().orEmpty()

fun ContactRecord.toBridgeResource(collectionId: String = "contacts"): BridgeResource {
    val fields = linkedMapOf<String, List<BridgeOccurrence>>()
    if (displayName.isNotBlank()) fields["FN"] = listOf(BridgeOccurrence(displayName))
    if (givenName.isNotBlank() || familyName.isNotBlank()) {
        fields["N"] = listOf(BridgeOccurrence(listOf(familyName, givenName, "", "", "").joinToString(";")))
    }
    if (phones.isNotEmpty()) fields["TEL"] = phones.map { value ->
        BridgeOccurrence(value.value, value.label.takeIf { it.isNotBlank() }?.let { mapOf("TYPE" to listOf(it)) } ?: emptyMap())
    }
    if (emails.isNotEmpty()) fields["EMAIL"] = emails.map { value ->
        BridgeOccurrence(value.value, value.label.takeIf { it.isNotBlank() }?.let { mapOf("TYPE" to listOf(it)) } ?: emptyMap())
    }
    if (notes.isNotEmpty()) fields["NOTE"] = notes.map(::BridgeOccurrence)
    if (groups.isNotEmpty()) fields["CATEGORIES"] = groups.sorted().map(::BridgeOccurrence)
    val revision = canonicalRevision.toLongOrNull() ?: 0L
    return BridgeResource(
        collectionId = collectionId,
        resourceId = "contact:$anytypeObjectId",
        kind = "contact",
        anytypeObjectId = anytypeObjectId,
        davUid = "contact:$anytypeObjectId",
        document = BridgeDocument(fields),
        revision = revision,
    ).validate()
}

fun BridgeResource.toContactRecord(): ContactRecord {
    require(kind == "contact") { "bridge resource is not a contact" }
    val fields = document.fields
    val name = fields["N"]?.firstOrNull()?.value?.split(';') ?: emptyList()
    val given = name.getOrNull(1).orEmpty()
    val family = name.firstOrNull().orEmpty()
    val display = fields["FN"]?.firstOrNull()?.value
        ?: listOf(given, family).filter { it.isNotBlank() }.joinToString(" ")
    return ContactRecord(
        anytypeObjectId = anytypeObjectId,
        displayName = display,
        givenName = given,
        familyName = family,
        phones = fields["TEL"].orEmpty().map { LabeledValue(it.value, it.label()) },
        emails = fields["EMAIL"].orEmpty().map { LabeledValue(it.value, it.label()) },
        notes = fields["NOTE"].orEmpty().values(),
        groups = fields["CATEGORIES"].orEmpty().values().toSet(),
        canonicalRevision = revision.toString(),
    )
}

/** Provider-facing metadata; row IDs are operational and never canonical identity. */
data class ContactProjectionState(
    val sourceId: String,
    val rawContactId: Long? = null,
    val lastProjectedHash: String? = null,
    val lastObservedHash: String? = null,
    val tombstoneRevision: String? = null,
)

sealed interface ContactProjectionOperation {
    data class UpsertRawContact(
        val sourceId: String,
        val accountName: String,
        val accountType: String,
        val displayName: String,
    ) : ContactProjectionOperation

    data class ReplaceData(val rows: List<DataRow>) : ContactProjectionOperation
    data class EnsureGroups(val names: List<String>) : ContactProjectionOperation
    data class ReplaceMembership(val groupNames: List<String>) : ContactProjectionOperation
    data class Tombstone(val sourceId: String, val canonicalRevision: String) : ContactProjectionOperation
}

data class DataRow(
    val mimeType: String,
    val value: String,
    val label: String,
    val data2: String? = null,
    val data3: String? = null,
)

/** Deterministic ContactsContract projection planning; this class performs no writes. */
class ContactsProjectionMapper(
    private val accountName: String,
    private val accountType: String,
) {
    init {
        require(accountName.isNotBlank()) { "account name must not be blank" }
        require(accountType.isNotBlank()) { "account type must not be blank" }
    }

    fun sourceId(anytypeObjectId: String): String {
        require(anytypeObjectId.isNotBlank()) { "Anytype object ID must not be blank" }
        return "android/$accountType/$accountName/$anytypeObjectId"
    }

    fun plan(contact: ContactRecord, previous: ContactProjectionState? = null): List<ContactProjectionOperation> {
        val source = sourceId(contact.anytypeObjectId)
        require(previous == null || previous.sourceId == source) { "projection identity mismatch" }
        if (contact.deleted) {
            return listOf(ContactProjectionOperation.Tombstone(source, contact.canonicalRevision))
        }
        val rows = dataRows(contact)
        return listOf(
            ContactProjectionOperation.UpsertRawContact(source, accountName, accountType, contact.displayName),
            ContactProjectionOperation.ReplaceData(rows),
            ContactProjectionOperation.EnsureGroups(contact.groups.sorted()),
            ContactProjectionOperation.ReplaceMembership(contact.groups.sorted()),
        )
    }

    fun projectionHash(contact: ContactRecord): String {
        val canonical = buildString {
            append(contact.displayName).append('\u0000')
            append(contact.givenName).append('\u0000').append(contact.familyName).append('\u0000')
            dataRows(contact).forEach {
                append(it.mimeType).append('|').append(it.label).append('|').append(it.value)
                    .append('|').append(it.data2).append('|').append(it.data3).append('\u0000')
            }
            contact.groups.toSortedSet().forEach { append("GROUP|").append(it).append('\u0000') }
        }
        return MessageDigest.getInstance("SHA-256").digest(canonical.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    private fun dataRows(contact: ContactRecord): List<DataRow> = buildList {
        if (contact.givenName.isNotEmpty() || contact.familyName.isNotEmpty()) {
            add(DataRow(
                ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE,
                contact.displayName,
                "",
                data2 = contact.familyName,
                data3 = contact.givenName,
            ))
        }
        contact.phones.mapTo(this) { DataRow(ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE, it.value, it.label) }
        contact.emails.mapTo(this) { DataRow(ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE, it.value, it.label) }
        contact.notes.mapTo(this) { DataRow(ContactsContract.CommonDataKinds.Note.CONTENT_ITEM_TYPE, it, "") }
    }.sortedWith(compareBy<DataRow> { it.mimeType }.thenBy { it.label }.thenBy { it.value })
}
