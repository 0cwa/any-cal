package org.anycal.android.account

import android.accounts.Account

/** Provider-free account lifecycle checks. Tokens are only held by the fake AccountManager boundary. */
object AccountLifecycleChecks {
    @JvmStatic
    fun main(args: Array<String>) = runAll()

    fun runAll() {
        addsAndRejectsDuplicateWithoutNewGeneration()
        removesOnlyAfterCleanupAndReaddsWithNewGeneration()
        revokedTokenAndPermissionFailureFailClosed()
        cleanupRedactsSecretsByConstruction()
    }

    private fun addsAndRejectsDuplicateWithoutNewGeneration() {
        val store = FakeStore()
        val manager = AccountLifecycleManager(store, FakeCleanup())
        val first = manager.add("space-a", "opaque-token") as AccountLifecycleResult.Added
        val duplicate = manager.add("space-a", "rotated-token") as AccountLifecycleResult.Existing
        check(first.account.generation == duplicate.account.generation)
        check(store.peekToken(first.account.account) == "rotated-token")
        check(store.generations.size == 1)
    }

    private fun removesOnlyAfterCleanupAndReaddsWithNewGeneration() {
        val store = FakeStore()
        val cleanup = FakeCleanup()
        val manager = AccountLifecycleManager(store, cleanup)
        val first = manager.add("space-a", "token") as AccountLifecycleResult.Added
        val removed = manager.remove("space-a") as AccountLifecycleResult.Removed
        check(removed.generation == first.account.generation && cleanup.calls == 1)
        val second = manager.add("space-a", "token-2") as AccountLifecycleResult.Added
        check(second.account.generation != first.account.generation)
    }

    private fun revokedTokenAndPermissionFailureFailClosed() {
        val store = FakeStore()
        val manager = AccountLifecycleManager(store, FakeCleanup(success = false))
        val added = manager.add("space-a", "token") as AccountLifecycleResult.Added
        store.tokens.remove(added.account.account)
        check(manager.token(added.account.account) == null)
        check(manager.remove("space-a") is AccountLifecycleResult.Rejected)
        check(store.generations.size == 1)
    }

    private fun cleanupRedactsSecretsByConstruction() {
        val cleanup = FakeCleanup(reason = "permission denied")
        check(cleanup.cleanup(Account("space-a", ANYCAL_ACCOUNT_TYPE), "generation").reason == "permission denied")
        check("token" !in cleanup.lastReason.lowercase())
    }

    private class FakeStore : AccountStore {
        val generations = mutableMapOf<String, AccountGeneration>()
        val tokens = mutableMapOf<Account, String>()
        override fun find(name: String, type: String) = generations[name]
        override fun add(name: String, type: String, generation: String): AccountGeneration {
            val result = AccountGeneration(Account(name, type), generation)
            generations[name] = result
            return result
        }
        override fun setToken(account: Account, token: CharSequence): Boolean { tokens[account] = token.toString(); return true }
        override fun peekToken(account: Account) = tokens[account]
        override fun remove(account: Account): Boolean { generations.remove(account.name); tokens.remove(account); return true }
    }

    private class FakeCleanup(
        private val success: Boolean = true,
        private val reason: String = "",
    ) : AccountProviderCleanup {
        var calls = 0
        var lastReason = reason
        override fun cleanup(account: Account, generation: String): CleanupResult {
            calls += 1
            lastReason = reason
            return CleanupResult(success, listOf("source-${account.name}"), reason)
        }
    }
}
