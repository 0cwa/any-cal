package org.anycal.android.account

import android.accounts.Account
import android.accounts.AccountManager
import android.content.Context
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.core.content.ContextCompat
import android.Manifest
import android.content.pm.PackageManager

const val ANYCAL_ACCOUNT_TYPE = "org.anycal.account"
const val ANYCAL_ACCOUNT_NAME_KEY = "org.anycal.account_name"
const val ANYCAL_ACCOUNT_TOKEN_KEY = "org.anycal.auth_token"
const val ANYCAL_ACCOUNT_GENERATION_KEY = "org.anycal.account_generation"

data class AccountGeneration(val account: Account, val generation: String)

sealed interface AccountLifecycleResult {
    data class Added(val account: AccountGeneration) : AccountLifecycleResult
    data class Existing(val account: AccountGeneration) : AccountLifecycleResult
    data class Removed(val generation: String, val tombstones: List<String>) : AccountLifecycleResult
    data class Rejected(val reason: String) : AccountLifecycleResult
}

interface AccountStore {
    fun find(name: String, type: String): AccountGeneration?
    fun add(name: String, type: String, generation: String): AccountGeneration?
    fun setToken(account: Account, token: CharSequence): Boolean
    fun peekToken(account: Account): String?
    fun remove(account: Account): Boolean
}

interface AccountProviderCleanup {
    fun cleanup(account: Account, generation: String): CleanupResult
}

data class CleanupResult(val success: Boolean, val tombstones: List<String> = emptyList(), val reason: String? = null)

/** Account lifecycle policy. Tokens are handed directly to AccountManager and never
 * placed in generation metadata, logs, exceptions, or provider rows. */
class AccountLifecycleManager(
    private val store: AccountStore,
    private val cleanup: AccountProviderCleanup,
    private val accountType: String = ANYCAL_ACCOUNT_TYPE,
) {
    fun add(name: String, token: CharSequence?): AccountLifecycleResult {
        if (name.isBlank()) return AccountLifecycleResult.Rejected("account name is required")
        if (token.isNullOrEmpty()) return AccountLifecycleResult.Rejected("credential handoff is required")
        val existing = store.find(name, accountType)
        if (existing != null) {
            if (!store.setToken(existing.account, token)) return AccountLifecycleResult.Rejected("credential update failed")
            return AccountLifecycleResult.Existing(existing)
        }
        val generation = java.util.UUID.randomUUID().toString()
        val added = store.add(name, accountType, generation)
            ?: return AccountLifecycleResult.Rejected("account creation failed")
        if (!store.setToken(added.account, token)) {
            store.remove(added.account)
            return AccountLifecycleResult.Rejected("credential handoff failed")
        }
        return AccountLifecycleResult.Added(added)
    }

    fun remove(name: String): AccountLifecycleResult {
        val existing = store.find(name, accountType)
            ?: return AccountLifecycleResult.Rejected("account not found")
        val cleaned = cleanup.cleanup(existing.account, existing.generation)
        if (!cleaned.success) return AccountLifecycleResult.Rejected(cleaned.reason ?: "provider cleanup failed")
        if (!store.remove(existing.account)) return AccountLifecycleResult.Rejected("account removal failed")
        return AccountLifecycleResult.Removed(existing.generation, cleaned.tombstones)
    }

    fun token(account: Account): String? = store.peekToken(account)
}

class AccountManagerStore(private val context: Context) : AccountStore {
    private val manager = AccountManager.get(context)

    override fun find(name: String, type: String): AccountGeneration? =
        manager.getAccountsByType(type).firstOrNull { it.name == name }?.let { account ->
            manager.getUserData(account, ANYCAL_ACCOUNT_GENERATION_KEY)?.let { AccountGeneration(account, it) }
        }

    override fun add(name: String, type: String, generation: String): AccountGeneration? {
        val account = Account(name, type)
        if (!manager.addAccountExplicitly(account, null, android.os.Bundle().apply {
                putString(ANYCAL_ACCOUNT_GENERATION_KEY, generation)
            })) return null
        return AccountGeneration(account, generation)
    }

    override fun setToken(account: Account, token: CharSequence): Boolean {
        manager.setAuthToken(account, "anycal", token.toString())
        return true
    }

    override fun peekToken(account: Account): String? = manager.peekAuthToken(account, "anycal")

    override fun remove(account: Account): Boolean = manager.removeAccountExplicitly(account)
}

/** Deletes only rows owned by the Any-Cal account. Missing runtime permissions fail closed. */
class ContentResolverAccountCleanup(private val context: Context) : AccountProviderCleanup {
    override fun cleanup(account: Account, generation: String): CleanupResult {
        if (!granted(Manifest.permission.READ_CONTACTS) || !granted(Manifest.permission.WRITE_CONTACTS) ||
            !granted(Manifest.permission.READ_CALENDAR) || !granted(Manifest.permission.WRITE_CALENDAR)
        ) return CleanupResult(false, reason = "provider cleanup permissions are missing")
        val resolver = context.contentResolver
        val tombstones = mutableListOf<String>()
        resolver.query(
            ContactsContract.RawContacts.CONTENT_URI,
            arrayOf(ContactsContract.RawContacts.SOURCE_ID),
            "${ContactsContract.RawContacts.ACCOUNT_NAME}=? AND ${ContactsContract.RawContacts.ACCOUNT_TYPE}=?",
            arrayOf(account.name, account.type), null,
        )?.use { cursor -> while (cursor.moveToNext()) cursor.getString(0)?.let { tombstones += it } }
        val contactUri = ContactsContract.RawContacts.CONTENT_URI.buildUpon()
            .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
            .appendQueryParameter(ContactsContract.RawContacts.ACCOUNT_NAME, account.name)
            .appendQueryParameter(ContactsContract.RawContacts.ACCOUNT_TYPE, account.type).build()
        resolver.delete(contactUri, "${ContactsContract.RawContacts.ACCOUNT_NAME}=? AND ${ContactsContract.RawContacts.ACCOUNT_TYPE}=?", arrayOf(account.name, account.type))
        val calendarUri = CalendarContract.Calendars.CONTENT_URI.buildUpon()
            .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
            .appendQueryParameter(CalendarContract.Calendars.ACCOUNT_NAME, account.name)
            .appendQueryParameter(CalendarContract.Calendars.ACCOUNT_TYPE, account.type).build()
        resolver.delete(calendarUri, "${CalendarContract.Calendars.ACCOUNT_NAME}=? AND ${CalendarContract.Calendars.ACCOUNT_TYPE}=?", arrayOf(account.name, account.type))
        return CleanupResult(true, tombstones)
    }

    private fun granted(permission: String) = ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
}
