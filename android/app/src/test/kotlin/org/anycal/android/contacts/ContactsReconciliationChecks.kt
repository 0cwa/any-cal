package org.anycal.android.contacts

import org.anycal.android.sync.ProjectionBinding
import org.anycal.android.sync.ProjectionOperationContext

/** Deterministic fake-provider checks for observer reconciliation; no ContentResolver access. */
object ContactsReconciliationChecks {
    @JvmStatic
    fun main(args: Array<String>) = runAll()

    fun runAll() {
        ignoresProjectionEcho()
        emitsExternalEditAndDelete()
        reportsConflictAndRowRepair()
        resumesInOrderAndStopsAfterAccountRemoval()
    }

    private val account = "space-a"
    private val type = "invalid.example.anycal"

    private fun source(id: String) = "android/$type/$account/$id"
    private fun record(id: String) = ContactRecord(id, id, phones = listOf(LabeledValue("+46", "cell")))
    private fun binding(id: String, row: Long = 1, hash: String = "h") = ProjectionBinding(
        account, type, "com.android.contacts", id, row, source(id), hash, hash, "1", null, null, hash,
    )

    private fun engine(
        snapshots: List<ContactProviderSnapshot>,
        saved: MutableMap<String, ProjectionBinding> = mutableMapOf(),
        operationFor: (String) -> ProjectionOperationContext? = { null },
        active: () -> Boolean = { true },
        publish: (List<ContactOutboundChange>) -> ContactPublishResult = { ContactPublishResult.Success },
    ): ContactsReconciliationEngine {
        val repo = object : ContactBindingRepository {
            override fun get(canonicalId: String) = saved[canonicalId]
            override fun save(binding: ProjectionBinding) { saved[binding.canonicalId] = binding }
        }
        val provider = object : ContactsProviderReader {
            override fun ownedSnapshots(sourceIds: Set<String>, limit: Int) = snapshots.filter { it.sourceId in sourceIds }.take(limit)
        }
        return ContactsReconciliationEngine(account, type, repo, provider, object : ContactsOutboundSource {
            override fun publish(changes: List<ContactOutboundChange>) = publish(changes)
        }, active, operationFor)
    }

    private fun ignoresProjectionEcho() {
        val saved = mutableMapOf("a" to binding("a"))
        val snapshot = ContactProviderSnapshot("a", source("a"), 1, "h", record("a"), true, account, type)
        val result = engine(listOf(snapshot), saved).reconcile(setOf(source("a")), ContactsReconciliationCheckpoint(generation = "g1"))
        check(result.emitted.isEmpty() && result.conflicts.isEmpty())
    }

    private fun emitsExternalEditAndDelete() {
        val saved = mutableMapOf("a" to binding("a"), "b" to binding("b"))
        val edit = ContactProviderSnapshot("a", source("a"), 1, "new", record("a"), true, account, type)
        val deleted = ContactProviderSnapshot("b", source("b"), null, null, null, false, account, type)
        val emitted = mutableListOf<ContactOutboundChange>()
        val result = engine(listOf(edit, deleted), saved, publish = { emitted += it; ContactPublishResult.Success })
            .reconcile(setOf(source("a"), source("b")), ContactsReconciliationCheckpoint(generation = "g1"))
        check(result.emitted.size == 2 && emitted.any { it is ContactOutboundChange.Upsert } && emitted.any { it is ContactOutboundChange.Tombstone })
    }

    private fun reportsConflictAndRowRepair() {
        val saved = mutableMapOf("a" to binding("a"), "b" to binding("b"))
        val conflict = ContactProviderSnapshot("a", source("a"), 1, "new", record("a"), true, account, type)
        val recreated = ContactProviderSnapshot("b", source("b"), 9, "h", record("b"), true, account, type)
        val result = engine(listOf(conflict, recreated), saved, operationFor = {
            if (it == "a") ProjectionOperationContext("op", source("a"), "old", "expected", "2") else null
        }).reconcile(setOf(source("a"), source("b")), ContactsReconciliationCheckpoint(generation = "g1"))
        check(result.conflicts.single().canonicalId == "a" && result.repairs == listOf("b"))
    }

    private fun resumesInOrderAndStopsAfterAccountRemoval() {
        val saved = mutableMapOf("b" to binding("b"), "c" to binding("c"))
        val snapshots = listOf(
            ContactProviderSnapshot("b", source("b"), 2, "new", record("b"), true, account, type),
            ContactProviderSnapshot("c", source("c"), 3, "new", record("c"), true, account, type),
        )
        // Checkpoints store the same full opaque source identity used for ordering;
        // a bare canonical ID must never be mixed into this cursor.
        val result = engine(snapshots, saved).reconcile(
            setOf(source("b"), source("c")),
            ContactsReconciliationCheckpoint(source("b"), "g1"),
            limit = 1,
        )
        check(result.checkpoint.lastSourceId == source("c"))
        val stopped = engine(snapshots, saved, active = { false }).reconcile(setOf(source("c")), ContactsReconciliationCheckpoint(generation = "g1"))
        check(stopped.stopped && stopped.emitted.isEmpty())
    }
}
