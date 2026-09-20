package org.anycal.android.contacts

import org.anycal.android.BridgeDecision
import org.anycal.android.BridgeDecisionResult
import org.anycal.android.BridgeRequest
import org.anycal.android.BridgeResponse
import org.anycal.android.ProviderCapabilities
import org.anycal.android.RustSyncBridge
import org.anycal.android.SyncResult
import org.anycal.android.sync.ProjectionBinding

/** Provider-free callback acceptance checks; no ContentResolver or Anytype endpoint is used. */
object ContactsSyncCallbackChecks {
    @JvmStatic
    fun main(args: Array<String>) {
        runAll()
    }

    fun runAll() {
        plansInStableOrderAndIsIdempotent()
        rejectsWrongScopeAndPermissions()
        rejectsBindingIdentityMismatch()
        persistsOnlyAfterApplyAndHandlesTombstone()
        rejectsTombstoneWithoutRevision()
    }

    private val request = BridgeRequest(
        accountName = "space-a", accountType = "invalid.example.anycal",
        authority = "com.android.contacts", checkpoint = null,
        resources = listOf("b", "a"), tombstones = emptyList(),
    )

    private fun contact(id: String, revision: String = "1") = ContactRecord(
        anytypeObjectId = id, displayName = id, phones = listOf(LabeledValue("+46", "cell")),
        canonicalRevision = revision,
    )

    private fun fakeBridge(ids: List<String>, decision: BridgeDecision = BridgeDecision.UPSERT) =
        object : RustSyncBridge {
            override fun syncOnce(capabilities: ProviderCapabilities): SyncResult = SyncResult.NotLinked
            override fun sync(request: BridgeRequest) = BridgeResponse(
                request.checkpoint,
                ids.sorted().map { BridgeDecisionResult(it, decision) },
                null,
            )
        }

    private fun callback(gateway: FakeGateway, repo: FakeRepo, bridge: RustSyncBridge = fakeBridge(listOf("a", "b"))) =
        ContactsSyncCallback("space-a", "invalid.example.anycal", gateway, repo, bridge)

    private fun plansInStableOrderAndIsIdempotent() {
        val gateway = FakeGateway()
        val repo = FakeRepo()
        val callback = callback(gateway, repo)
        val first = callback.sync(request, listOf(contact("b"), contact("a")), apply = false)
        val second = callback.sync(request, listOf(contact("a"), contact("b")), apply = false)
        check(first.operations.map { it.canonicalId } == second.operations.map { it.canonicalId })
        check(first.operations.map { it.canonicalId }.first() == "a")
        check(first.operations.take(4).map { it.operation::class } == listOf(
            ContactProjectionOperation.UpsertRawContact::class,
            ContactProjectionOperation.ReplaceData::class,
            ContactProjectionOperation.EnsureGroups::class,
            ContactProjectionOperation.ReplaceMembership::class,
        ))
        check(first.applied == 0)
    }

    private fun rejectsWrongScopeAndPermissions() {
        val denied = FakeGateway(ContactsCapabilities(true, true, false))
        check(runCatching { callback(denied, FakeRepo()).sync(request, listOf(contact("a")), false) }.isFailure)
        val wrong = request.copy(accountName = "other")
        check(runCatching { callback(FakeGateway(), FakeRepo()).sync(wrong, listOf(contact("a")), false) }.isFailure)
    }

    private fun rejectsBindingIdentityMismatch() {
        val repo = FakeRepo()
        repo.saved += ProjectionBinding(
            "space-a", "invalid.example.anycal", "com.android.contacts", "a", 9L,
            "android/other-account/a", null, null, null, null, null, null,
        )
        check(runCatching { callback(FakeGateway(), repo).sync(request.copy(resources = listOf("a")), listOf(contact("a")), false) }.isFailure)
    }

    private fun persistsOnlyAfterApplyAndHandlesTombstone() {
        val gateway = FakeGateway()
        val repo = FakeRepo()
        val cb = callback(gateway, repo, fakeBridge(listOf("a"), BridgeDecision.ARCHIVE))
        val tombstone = request.copy(resources = emptyList(), tombstones = listOf(
            org.anycal.android.BridgeTombstone("r-a", "a", 2),
        ))
        val preview = cb.sync(tombstone, emptyList(), apply = false)
        check(repo.saved.isEmpty() && preview.operations.single().operation is ContactProjectionOperation.Tombstone)
        val applied = cb.sync(tombstone, emptyList(), apply = true)
        check(applied.applied == 1 && repo.saved.single().canonicalRevision == "2" &&
            repo.saved.single().tombstoneRevision == "2")
    }

    private fun rejectsTombstoneWithoutRevision() {
        val tombstone = request.copy(resources = emptyList(), tombstones = listOf(
            org.anycal.android.BridgeTombstone("r-a", "a", 0),
        ))
        check(runCatching { callback(FakeGateway(), FakeRepo(), fakeBridge(listOf("a"), BridgeDecision.ARCHIVE)).sync(tombstone, emptyList(), false) }.isFailure)
    }

    private class FakeGateway(
        private val caps: ContactsCapabilities = ContactsCapabilities(true, true, true),
    ) : ContactsProviderGateway {
        var applied = 0
        override fun capabilities() = caps
        override fun observe(sourceId: String) = ContactProviderObservation(null, null, false)
        override fun apply(operation: ContactProjectionOperation): ContactProviderMutation {
            applied += 1
            return ContactProviderMutation(applied.toLong(), "hash-$applied")
        }
    }

    private class FakeRepo : ContactBindingRepository {
        val saved = mutableListOf<ProjectionBinding>()
        override fun get(canonicalId: String) = saved.lastOrNull { it.canonicalId == canonicalId }
        override fun save(binding: ProjectionBinding) { saved.removeAll { it.canonicalId == binding.canonicalId }; saved += binding }
    }
}
