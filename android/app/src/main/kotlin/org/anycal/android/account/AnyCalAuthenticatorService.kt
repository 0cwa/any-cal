package org.anycal.android.account

import android.accounts.AbstractAccountAuthenticator
import android.accounts.Account
import android.accounts.AccountManager
import android.accounts.AccountAuthenticatorResponse
import android.content.Context
import android.os.Bundle

/** Account authenticator. Credential acquisition remains caller-owned; this service only
 * receives an opaque handoff and stores it through AccountManager. */
class AnyCalAuthenticatorService : android.app.Service() {
    private lateinit var authenticator: Authenticator

    override fun onCreate() {
        super.onCreate()
        authenticator = Authenticator(this)
    }

    override fun onBind(intent: android.content.Intent?) = authenticator.iBinder
}

private class Authenticator(private val context: Context) : AbstractAccountAuthenticator(context) {
    private val lifecycle = AccountLifecycleManager(AccountManagerStore(context), ContentResolverAccountCleanup(context))

    override fun addAccount(
        response: AccountAuthenticatorResponse?,
        accountType: String?,
        authTokenType: String?,
        requiredFeatures: Array<out String>?,
        options: Bundle?,
    ): Bundle {
        if (accountType != ANYCAL_ACCOUNT_TYPE) return error(response, "unsupported account type")
        val name = options?.getString(ANYCAL_ACCOUNT_NAME_KEY)
        val token = options?.getCharSequence(ANYCAL_ACCOUNT_TOKEN_KEY)
        return when (val result = lifecycle.add(name ?: "", token)) {
            is AccountLifecycleResult.Added -> accountResult(result.account.account)
            is AccountLifecycleResult.Existing -> accountResult(result.account.account)
            is AccountLifecycleResult.Rejected -> error(response, result.reason)
            is AccountLifecycleResult.Removed -> error(response, "account was removed")
        }
    }

    override fun confirmCredentials(
        response: AccountAuthenticatorResponse?,
        account: Account?,
        options: Bundle?,
    ): Bundle = if (account != null && lifecycle.token(account) != null) {
        Bundle().apply { putBoolean(AccountManager.KEY_BOOLEAN_RESULT, true) }
    } else error(response, "credential unavailable")

    override fun editProperties(response: AccountAuthenticatorResponse?, accountType: String?): Bundle =
        error(response, "account properties are not editable here")

    override fun getAuthToken(
        response: AccountAuthenticatorResponse?,
        account: Account?,
        authTokenType: String?,
        options: Bundle?,
    ): Bundle {
        if (account == null || account.type != ANYCAL_ACCOUNT_TYPE) return error(response, "unsupported account")
        val token = lifecycle.token(account) ?: return error(response, "credential unavailable")
        return Bundle().apply {
            putString(AccountManager.KEY_ACCOUNT_NAME, account.name)
            putString(AccountManager.KEY_ACCOUNT_TYPE, account.type)
            putString(AccountManager.KEY_AUTHTOKEN, token)
        }
    }

    override fun getAuthTokenLabel(authTokenType: String?): String? = null

    override fun hasFeatures(
        response: AccountAuthenticatorResponse?,
        account: Account?,
        features: Array<out String>?,
    ): Bundle = Bundle().apply { putBoolean(AccountManager.KEY_BOOLEAN_RESULT, false) }

    override fun updateCredentials(
        response: AccountAuthenticatorResponse?,
        account: Account?,
        authTokenType: String?,
        options: Bundle?,
    ): Bundle {
        if (account == null || account.type != ANYCAL_ACCOUNT_TYPE) return error(response, "unsupported account")
        val token = options?.getCharSequence(ANYCAL_ACCOUNT_TOKEN_KEY)
            ?: return error(response, "credential handoff is required")
        return if (AccountManagerStore(context).setToken(account, token)) accountResult(account) else error(response, "credential update failed")
    }

    override fun getAccountRemovalAllowed(
        response: AccountAuthenticatorResponse?,
        account: Account?,
    ): Bundle {
        if (account == null || account.type != ANYCAL_ACCOUNT_TYPE) return error(response, "unsupported account")
        return when (lifecycle.remove(account.name)) {
            is AccountLifecycleResult.Removed -> Bundle().apply { putBoolean(AccountManager.KEY_BOOLEAN_RESULT, true) }
            is AccountLifecycleResult.Rejected -> error(response, "account cleanup failed")
            else -> error(response, "account removal failed")
        }
    }

    private fun accountResult(account: Account) = Bundle().apply {
        putString(AccountManager.KEY_ACCOUNT_NAME, account.name)
        putString(AccountManager.KEY_ACCOUNT_TYPE, account.type)
    }

    private fun error(response: AccountAuthenticatorResponse?, reason: String): Bundle = Bundle().also {
        response?.onError(AccountManager.ERROR_CODE_INVALID_RESPONSE, reason)
    }
}
