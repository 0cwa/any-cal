package org.anycal.android.contacts

import android.Manifest
import android.content.Context
import android.content.ContentValues
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.ContactsContract
import androidx.core.content.ContextCompat

data class ContactsCapabilities(
    val authorityAvailable: Boolean,
    val canRead: Boolean,
    val canWrite: Boolean,
) {
    val writable: Boolean get() = authorityAvailable && canRead && canWrite
}

/** Capability and URI boundary for the account-owned ContactsContract adapter. */
class ContactsContractAdapter(private val context: Context) {
    fun capabilities(): ContactsCapabilities {
        val pm = context.packageManager
        return ContactsCapabilities(
            authorityAvailable = pm.resolveContentProvider(ContactsContract.AUTHORITY, 0) != null,
            canRead = granted(Manifest.permission.READ_CONTACTS),
            canWrite = granted(Manifest.permission.WRITE_CONTACTS),
        )
    }

    /** Writes must use this URI so provider changes are not echoed as local edits. */
    fun asSyncAdapter(uri: Uri, accountName: String, accountType: String): Uri {
        require(accountName.isNotBlank()) { "account name must not be blank" }
        require(accountType.isNotBlank()) { "account type must not be blank" }
        return uri.buildUpon()
            .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
            .appendQueryParameter(ContactsContract.RawContacts.ACCOUNT_NAME, accountName)
            .appendQueryParameter(ContactsContract.RawContacts.ACCOUNT_TYPE, accountType)
            .build()
    }

    fun requireWritable() {
        check(capabilities().writable) { "ContactsContract unavailable or contacts permissions are missing" }
    }

    private fun granted(permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
}

/** ContentResolver boundary for the callback. Operations are applied in the mapper's
 * order and every mutation is account-owned and sent as a sync-adapter write. */
class ContentResolverContactsGateway(
    private val context: Context,
    private val accountName: String,
    private val accountType: String,
) : ContactsProviderGateway {
    private val adapter = ContactsContractAdapter(context)
    private val reader = ContentResolverContactsReader(context, accountName, accountType)
    private var rawContactId: Long? = null
    private var rawContactSourceId: String? = null

    override fun capabilities() = adapter.capabilities()

    override fun observe(sourceId: String): ContactProviderObservation {
        adapter.requireWritable()
        val snapshot = reader.ownedSnapshots(setOf(sourceId), 1).single()
        return ContactProviderObservation(snapshot.rowId, snapshot.hash, snapshot.present)
    }

    override fun apply(operation: ContactProjectionOperation): ContactProviderMutation {
        adapter.requireWritable()
        return when (operation) {
            is ContactProjectionOperation.UpsertRawContact -> {
                check(operation.accountName == accountName && operation.accountType == accountType) {
                    "contact operation account ownership mismatch"
                }
                val existing = observe(operation.sourceId).rowId
                rawContactSourceId = operation.sourceId
                rawContactId = existing ?: context.contentResolver.insert(
                    adapter.asSyncAdapter(ContactsContract.RawContacts.CONTENT_URI, accountName, accountType),
                    ContentValues().apply {
                        put(ContactsContract.RawContacts.ACCOUNT_NAME, accountName)
                        put(ContactsContract.RawContacts.ACCOUNT_TYPE, accountType)
                        put(ContactsContract.RawContacts.SOURCE_ID, operation.sourceId)
                    },
                )?.lastPathSegment?.toLongOrNull()
                check(rawContactId != null) { "failed to create contact raw row" }
                ContactProviderMutation(rawContactId, null)
            }
            is ContactProjectionOperation.ReplaceData -> {
                check(rawContactSourceId != null) { "data operation has no current contact" }
                val rawId = checkNotNull(rawContactId) { "data operation has no raw contact" }
                val resolver = context.contentResolver
                resolver.delete(
                    adapter.asSyncAdapter(ContactsContract.Data.CONTENT_URI, accountName, accountType),
                    "${ContactsContract.Data.RAW_CONTACT_ID}=?",
                    arrayOf(rawId.toString()),
                )
                operation.rows.forEach { row ->
                    resolver.insert(
                        adapter.asSyncAdapter(ContactsContract.Data.CONTENT_URI, accountName, accountType),
                        ContentValues().apply {
                            put(ContactsContract.Data.RAW_CONTACT_ID, rawId)
                            put(ContactsContract.Data.MIMETYPE, row.mimeType)
                            put(ContactsContract.Data.DATA1, row.value)
                            if (row.mimeType == ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE) {
                                row.data2?.let { put(ContactsContract.Data.DATA2, it) }
                                row.data3?.let { put(ContactsContract.Data.DATA3, it) }
                            } else if (row.label.isNotBlank()) {
                                when (row.mimeType) {
                                    ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE -> putPhoneLabel(this, row.label)
                                    ContactsContract.CommonDataKinds.Email.CONTENT_ITEM_TYPE -> putEmailLabel(this, row.label)
                                    else -> put(ContactsContract.Data.DATA2, row.label)
                                }
                            }
                        },
                    ) ?: error("failed to insert contact data")
                }
                ContactProviderMutation(rawId, null)
            }
            is ContactProjectionOperation.EnsureGroups -> {
                check(rawContactSourceId != null) { "group operation has no current contact" }
                operation.names.forEach(::ensureGroup)
                ContactProviderMutation(rawContactId, null)
            }
            is ContactProjectionOperation.ReplaceMembership -> {
                check(rawContactSourceId != null) { "membership operation has no current contact" }
                val rawId = checkNotNull(rawContactId) { "membership operation has no raw contact" }
                val dataUri = adapter.asSyncAdapter(ContactsContract.Data.CONTENT_URI, accountName, accountType)
                context.contentResolver.delete(
                    dataUri,
                    "${ContactsContract.Data.RAW_CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
                    arrayOf(rawId.toString(), ContactsContract.CommonDataKinds.GroupMembership.CONTENT_ITEM_TYPE),
                )
                operation.groupNames.forEach { name ->
                    val groupId = ensureGroup(name)
                    context.contentResolver.insert(dataUri, ContentValues().apply {
                        put(ContactsContract.Data.RAW_CONTACT_ID, rawId)
                        put(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.GroupMembership.CONTENT_ITEM_TYPE)
                        put(ContactsContract.CommonDataKinds.GroupMembership.GROUP_ROW_ID, groupId)
                    }) ?: error("failed to insert contact group membership")
                }
                ContactProviderMutation(rawId, null)
            }
            is ContactProjectionOperation.Tombstone -> {
                // A prior contact's in-memory row must never be reused for a
                // tombstone belonging to another canonical source.
                val rawId = if (rawContactSourceId == operation.sourceId) {
                    rawContactId ?: observe(operation.sourceId).rowId
                } else {
                    observe(operation.sourceId).rowId
                }
                if (rawId != null) {
                    context.contentResolver.delete(
                        adapter.asSyncAdapter(ContactsContract.RawContacts.CONTENT_URI, accountName, accountType),
                        "${ContactsContract.RawContacts._ID}=?",
                        arrayOf(rawId.toString()),
                    )
                }
                if (rawContactSourceId == operation.sourceId) {
                    rawContactId = null
                    rawContactSourceId = null
                }
                ContactProviderMutation(rawId, null)
            }
        }
    }

    private fun ensureGroup(name: String): Long {
        require(name.isNotBlank()) { "contact group name must not be blank" }
        val selection = "${ContactsContract.Groups.ACCOUNT_NAME}=? AND ${ContactsContract.Groups.ACCOUNT_TYPE}=? AND ${ContactsContract.Groups.TITLE}=?"
        context.contentResolver.query(
            adapter.asSyncAdapter(ContactsContract.Groups.CONTENT_URI, accountName, accountType),
            arrayOf(ContactsContract.Groups._ID), selection, arrayOf(accountName, accountType, name), null,
        ).use { cursor ->
            if (cursor != null && cursor.moveToFirst()) return cursor.getLong(0)
        }
        return context.contentResolver.insert(
            adapter.asSyncAdapter(ContactsContract.Groups.CONTENT_URI, accountName, accountType),
            ContentValues().apply {
                put(ContactsContract.Groups.ACCOUNT_NAME, accountName)
                put(ContactsContract.Groups.ACCOUNT_TYPE, accountType)
                put(ContactsContract.Groups.TITLE, name)
            },
        )?.lastPathSegment?.toLongOrNull() ?: error("failed to create contact group")
    }

    private fun putPhoneLabel(values: ContentValues, label: String) {
        when (label.lowercase()) {
            "home" -> values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_HOME)
            "mobile", "cell" -> values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_MOBILE)
            "work" -> values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_WORK)
            "fax_work" -> values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_FAX_WORK)
            "fax_home" -> values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_FAX_HOME)
            "other" -> values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_OTHER)
            else -> {
                values.put(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_CUSTOM)
                values.put(ContactsContract.CommonDataKinds.Phone.LABEL, label)
            }
        }
    }

    private fun putEmailLabel(values: ContentValues, label: String) {
        when (label.lowercase()) {
            "home" -> values.put(ContactsContract.CommonDataKinds.Email.TYPE, ContactsContract.CommonDataKinds.Email.TYPE_HOME)
            "work" -> values.put(ContactsContract.CommonDataKinds.Email.TYPE, ContactsContract.CommonDataKinds.Email.TYPE_WORK)
            "other" -> values.put(ContactsContract.CommonDataKinds.Email.TYPE, ContactsContract.CommonDataKinds.Email.TYPE_OTHER)
            "mobile" -> values.put(ContactsContract.CommonDataKinds.Email.TYPE, ContactsContract.CommonDataKinds.Email.TYPE_MOBILE)
            else -> {
                values.put(ContactsContract.CommonDataKinds.Email.TYPE, ContactsContract.CommonDataKinds.Email.TYPE_CUSTOM)
                values.put(ContactsContract.CommonDataKinds.Email.LABEL, label)
            }
        }
    }
}
