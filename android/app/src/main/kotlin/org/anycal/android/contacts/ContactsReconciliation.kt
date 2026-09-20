package org.anycal.android.contacts

import android.database.ContentObserver
import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import org.anycal.android.sync.ProjectionDecision
import org.anycal.android.sync.ProjectionDecisions
import org.anycal.android.sync.ProjectionOperationContext

data class ContactProviderSnapshot(
    val canonicalId: String,
    val sourceId: String,
    val rowId: Long?,
    val hash: String?,
    val record: ContactRecord?,
    val present: Boolean,
    val accountName: String,
    val accountType: String,
)

sealed interface ContactOutboundChange {
    val canonicalId: String
    val sourceId: String

    data class Upsert(
        override val canonicalId: String,
        override val sourceId: String,
        val record: ContactRecord,
        val observedHash: String?,
    ) : ContactOutboundChange

    data class Tombstone(
        override val canonicalId: String,
        override val sourceId: String,
        val observedHash: String?,
    ) : ContactOutboundChange
}

data class ContactConflict(
    val canonicalId: String,
    val sourceId: String,
    val canonicalHash: String?,
    val providerHash: String?,
)

sealed interface ContactPublishResult {
    data object Success : ContactPublishResult
    data class Retryable(val reason: String) : ContactPublishResult
    data class Rejected(val reason: String) : ContactPublishResult
}

interface ContactsOutboundSource {
    fun publish(changes: List<ContactOutboundChange>): ContactPublishResult
}

interface ContactsProviderReader {
    fun ownedSnapshots(sourceIds: Set<String>, limit: Int): List<ContactProviderSnapshot>
}

data class ContactsReconciliationCheckpoint(val lastSourceId: String? = null, val generation: String)

data class ContactsReconciliationReport(
    val checkpoint: ContactsReconciliationCheckpoint,
    val emitted: List<ContactOutboundChange>,
    val conflicts: List<ContactConflict>,
    val repairs: List<String>,
    val retryable: String? = null,
    val stopped: Boolean = false,
)

/** Provider-local reconciliation. Observers only trigger this bounded query; they never
 * infer ownership from display fields and never write back to the provider. */
class ContactsReconciliationEngine(
    private val accountName: String,
    private val accountType: String,
    private val bindings: ContactBindingRepository,
    private val provider: ContactsProviderReader,
    private val outbound: ContactsOutboundSource,
    private val accountIsActive: () -> Boolean = { true },
    private val operationFor: (String) -> ProjectionOperationContext? = { null },
) {
    fun reconcile(
        sourceIds: Set<String>,
        checkpoint: ContactsReconciliationCheckpoint,
        limit: Int = 100,
    ): ContactsReconciliationReport {
        require(limit in 1..500) { "contact reconciliation batch must be 1..500" }
        if (!accountIsActive()) return ContactsReconciliationReport(checkpoint, emptyList(), emptyList(), emptyList(), stopped = true)
        require(checkpoint.generation.isNotBlank()) { "account generation is required" }
        val sourcePrefix = "android/$accountType/$accountName/"
        require(checkpoint.lastSourceId == null || checkpoint.lastSourceId.startsWith(sourcePrefix)) {
            "contact checkpoint must contain a full provider source ID"
        }
        val ordered = sourceIds.filter { it.isNotBlank() }.sorted()
            .filter { checkpoint.lastSourceId == null || it > checkpoint.lastSourceId }
            .take(limit).toSet()
        val snapshots = provider.ownedSnapshots(ordered, limit)
            .sortedWith(compareBy<ContactProviderSnapshot> { it.sourceId }.thenBy { it.canonicalId })
        val emitted = mutableListOf<ContactOutboundChange>()
        val conflicts = mutableListOf<ContactConflict>()
        val repairs = mutableListOf<String>()
        snapshots.forEach { snapshot ->
            require(snapshot.accountName == accountName && snapshot.accountType == accountType) {
                "provider snapshot account ownership mismatch"
            }
            val expectedSource = "android/$accountType/$accountName/${snapshot.canonicalId}"
            require(snapshot.sourceId == expectedSource) { "provider source identity mismatch" }
            val binding = bindings.get(snapshot.canonicalId)
            val decision = ProjectionDecisions.classify(
                binding,
                binding?.projectedHash,
                snapshot.hash,
                snapshot.present,
                deleteRequested = false,
                operation = operationFor(snapshot.canonicalId),
            )
            if (binding?.providerRowId != null && snapshot.present && snapshot.rowId != binding.providerRowId) {
                repairs += snapshot.canonicalId
                return@forEach
            }
            when (decision) {
                ProjectionDecision.NoOp, ProjectionDecision.ObserverEcho, ProjectionDecision.Replay -> Unit
                ProjectionDecision.Conflict -> conflicts += ContactConflict(
                    snapshot.canonicalId, snapshot.sourceId, binding?.projectedHash, snapshot.hash,
                )
                ProjectionDecision.ProviderRecreated -> if (!snapshot.present && binding != null) {
                    emitted += ContactOutboundChange.Tombstone(snapshot.canonicalId, snapshot.sourceId, snapshot.hash)
                } else {
                    repairs += snapshot.canonicalId
                }
                else -> if (snapshot.present && snapshot.record != null) {
                    emitted += ContactOutboundChange.Upsert(snapshot.canonicalId, snapshot.sourceId, snapshot.record, snapshot.hash)
                } else if (binding != null) {
                    emitted += ContactOutboundChange.Tombstone(snapshot.canonicalId, snapshot.sourceId, snapshot.hash)
                }
            }
        }
        if (conflicts.isNotEmpty()) {
            return ContactsReconciliationReport(checkpoint, emitted, conflicts, repairs)
        }
        if (emitted.isNotEmpty()) {
            when (val result = outbound.publish(emitted)) {
                ContactPublishResult.Success -> Unit
                is ContactPublishResult.Retryable -> return ContactsReconciliationReport(checkpoint, emitted, emptyList(), repairs, result.reason)
                is ContactPublishResult.Rejected -> return ContactsReconciliationReport(checkpoint, emptyList(), emptyList(), repairs, result.reason)
            }
        }
        val next = snapshots.lastOrNull()?.sourceId ?: checkpoint.lastSourceId
        return ContactsReconciliationReport(checkpoint.copy(lastSourceId = next), emitted, emptyList(), repairs)
    }
}

/** ContentObserver is a prompt only; reconciliation performs the authoritative query. */
class ContactsProviderObserver(
    handler: Handler = Handler(Looper.getMainLooper()),
    private val onChanged: (Uri?) -> Unit,
) : ContentObserver(handler) {
    override fun onChange(selfChange: Boolean, uri: Uri?) = onChanged(uri)
}

fun contactsOwnedUri(): Uri = ContactsContract.RawContacts.CONTENT_URI

/** Read-only provider adapter used by reconciliation. It queries only the supplied
 * source IDs and never adopts rows from another account. */
class ContentResolverContactsReader(
    private val context: Context,
    private val accountName: String,
    private val accountType: String,
) : ContactsProviderReader {
    private val mapper = ContactsProjectionMapper(accountName, accountType)

    override fun ownedSnapshots(sourceIds: Set<String>, limit: Int): List<ContactProviderSnapshot> {
        require(limit in 1..500)
        return sourceIds.sorted().take(limit).map { sourceId -> read(sourceId) }
    }

    private fun read(sourceId: String): ContactProviderSnapshot {
        val canonicalId = sourceId.removePrefix("android/$accountType/$accountName/")
        require(canonicalId.isNotBlank() && canonicalId != sourceId) { "foreign contact source identity" }
        val raw = context.contentResolver.query(
            ContactsContract.RawContacts.CONTENT_URI,
            arrayOf(ContactsContract.RawContacts._ID),
            "${ContactsContract.RawContacts.ACCOUNT_NAME}=? AND ${ContactsContract.RawContacts.ACCOUNT_TYPE}=? AND ${ContactsContract.RawContacts.SOURCE_ID}=?",
            arrayOf(accountName, accountType, sourceId), null,
        )?.use { cursor -> if (cursor.moveToFirst()) cursor.getLong(0) else null }
        if (raw == null) return ContactProviderSnapshot(canonicalId, sourceId, null, null, null, false, accountName, accountType)

        var display = ""
        var given = ""
        var family = ""
        val phones = mutableListOf<LabeledValue>()
        val emails = mutableListOf<LabeledValue>()
        val notes = mutableListOf<String>()
        val groups = mutableSetOf<String>()
        context.contentResolver.query(
            ContactsContract.Data.CONTENT_URI,
            arrayOf(ContactsContract.Data.MIMETYPE, ContactsContract.Data.DATA1, ContactsContract.Data.DATA2, ContactsContract.Data.DATA3),
            "${ContactsContract.Data.RAW_CONTACT_ID}=?",
            arrayOf(raw.toString()), null,
        )?.use { cursor ->
            while (cursor.moveToNext()) {
                val mime = cursor.getString(0) ?: continue
                val value = cursor.getString(1) ?: ""
                val data2 = cursor.getString(2) ?: ""
                val data3 = cursor.getString(3) ?: ""
                when (mime) {
                    ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE -> {
                        given = data3
                        family = data2
                        display = value.ifBlank { listOf(given, family).filter { it.isNotBlank() }.joinToString(" ") }
                    }
                    ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE -> phones += LabeledValue(value, phoneLabel(data2, data3))
                    ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE -> emails += LabeledValue(value, emailLabel(data2, data3))
                    ContactsContract.CommonDataKinds.Note.CONTENT_ITEM_TYPE -> notes += value
                    ContactsContract.CommonDataKinds.GroupMembership.CONTENT_ITEM_TYPE -> {
                        cursor.getString(3)?.toLongOrNull()?.let { groupId -> groups += groupTitle(groupId) }
                    }
                }
            }
        }
        val record = ContactRecord(canonicalId, display, given, family, phones, emails, notes, groups)
        return ContactProviderSnapshot(canonicalId, sourceId, raw, mapper.projectionHash(record), record, true, accountName, accountType)
    }

    private fun groupTitle(groupId: Long): String = context.contentResolver.query(
        ContactsContract.Groups.CONTENT_URI,
        arrayOf(ContactsContract.Groups.TITLE),
        "${ContactsContract.Groups._ID}=? AND ${ContactsContract.Groups.ACCOUNT_NAME}=? AND ${ContactsContract.Groups.ACCOUNT_TYPE}=?",
        arrayOf(groupId.toString(), accountName, accountType), null,
    )?.use { cursor -> if (cursor.moveToFirst()) cursor.getString(0) ?: "" else "" } ?: ""

    private fun phoneLabel(type: String, custom: String): String = when (type.toIntOrNull()) {
        ContactsContract.CommonDataKinds.Phone.TYPE_HOME -> "home"
        ContactsContract.CommonDataKinds.Phone.TYPE_MOBILE -> "mobile"
        ContactsContract.CommonDataKinds.Phone.TYPE_WORK -> "work"
        ContactsContract.CommonDataKinds.Phone.TYPE_FAX_WORK -> "fax_work"
        ContactsContract.CommonDataKinds.Phone.TYPE_FAX_HOME -> "fax_home"
        ContactsContract.CommonDataKinds.Phone.TYPE_OTHER -> "other"
        ContactsContract.CommonDataKinds.Phone.TYPE_CUSTOM -> custom
        else -> type
    }

    private fun emailLabel(type: String, custom: String): String = when (type.toIntOrNull()) {
        ContactsContract.CommonDataKinds.Email.TYPE_HOME -> "home"
        ContactsContract.CommonDataKinds.Email.TYPE_WORK -> "work"
        ContactsContract.CommonDataKinds.Email.TYPE_OTHER -> "other"
        ContactsContract.CommonDataKinds.Email.TYPE_MOBILE -> "mobile"
        ContactsContract.CommonDataKinds.Email.TYPE_CUSTOM -> custom
        else -> type
    }
}
