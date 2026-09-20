package org.anycal.android.tasks

import android.accounts.Account
import org.anycal.android.BridgeDecision
import org.anycal.android.BridgeRequest
import org.anycal.android.BridgeResponse
import org.anycal.android.RustSyncBridge
import org.anycal.android.sync.ProjectionBinding
import org.anycal.android.sync.ProjectionBindingStore

data class TasksOrgSyncReport(
    val created: Int,
    val updated: Int,
    val unchanged: Int,
    val deleted: Int,
    val skipped: Int,
)

/** Anytype -> Tasks.org projection. The provider is only touched after the
 * bridge authorizes each resource and the schema probe succeeds. */
class TasksOrgSyncRunner(
    private val account: Account,
    private val store: ProjectionBindingStore,
    private val gateway: TasksOrgProviderGateway,
    private val bridge: RustSyncBridge,
    private val generation: String,
    private val checkpointCommit: (org.anycal.android.BridgeCheckpoint) -> Unit = {},
) {
    fun run(): Result<TasksOrgSyncReport> = runCatching {
        gateway.schema().validate()
        require(generation.isNotBlank()) { "Tasks.org account generation is unavailable" }
        val checkpoint = store.checkpoint(account.name, account.type, TasksOrgAdapter.AUTHORITY, generation)
        val response = bridge.sync(
            BridgeRequest(account.name, account.type, TasksOrgAdapter.AUTHORITY, checkpoint, emptyList(), emptyList()),
        ).validate()
        check(response.error == null) { response.error?.message ?: "Tasks.org bridge pull failed" }
        val report = applyResponse(response)
        response.checkpoint?.let(checkpointCommit)
        response.checkpoint?.let { store.saveCheckpoint(account.name, account.type, TasksOrgAdapter.AUTHORITY, generation, it) }
        report
    }

    private fun applyResponse(response: BridgeResponse): TasksOrgSyncReport {
        val decisions = response.decisions.associateBy { it.resourceId }
        var created = 0
        var updated = 0
        var unchanged = 0
        var deleted = 0
        var skipped = 0
        response.resources.filter { it.kind == "task" }.forEach { resource ->
            val decision = decisions[resource.resourceId]?.decision
                ?: decisions[resource.anytypeObjectId]?.decision
                ?: error("bridge response omitted Tasks.org decision")
            if (decision != BridgeDecision.UPSERT && decision != BridgeDecision.NOOP) {
                skipped++
                return@forEach
            }
            if (decision == BridgeDecision.NOOP) {
                unchanged++
                return@forEach
            }
            val task = resource.toTasksOrgTask()
            val sourceId = sourceId(account, resource.anytypeObjectId)
            val existing = store.get(account.name, account.type, TasksOrgAdapter.AUTHORITY, resource.anytypeObjectId)
            val providerId = existing?.providerRowId
            if (providerId != null) {
                val current = gateway.read(providerId)
                val expected = existing.observedHash ?: existing.projectedHash
                if (current != null && (expected == null || current.hash() != expected)) {
                    // Numeric provider IDs are local and can be reused. Never
                    // overwrite a row that no longer matches our projection.
                    skipped++
                    return@forEach
                }
            }
            val resultingId = when {
                providerId == null -> {
                    created++
                    gateway.insert(task)
                }
                gateway.update(providerId, task) -> {
                    updated++
                    providerId
                }
                else -> {
                    created++
                    gateway.insert(task)
                }
            }
            store.upsert(ProjectionBinding(
                account.name, account.type, TasksOrgAdapter.AUTHORITY,
                resource.anytypeObjectId, resultingId, sourceId,
                projectedHash = task.providerHash(), observedHash = gateway.read(resultingId)?.hash(),
                canonicalRevision = resource.revision.toString(), tombstoneRevision = null,
                lastOperationId = "tasks-org:${resource.resourceId}:${resource.revision}",
                lastOperationPostHash = task.providerHash(),
            ))
        }
        response.tombstones.forEach { tombstone ->
            val binding = store.get(account.name, account.type, TasksOrgAdapter.AUTHORITY, tombstone.canonicalId)
                ?: return@forEach
            val providerId = binding.providerRowId ?: return@forEach
            val expected = binding.observedHash ?: binding.projectedHash ?: return@forEach
            if (!gateway.deleteIfOwned(providerId, expected)) {
                skipped++
                return@forEach
            }
            deleted++
            store.upsert(binding.copy(
                providerRowId = null,
                tombstoneRevision = tombstone.revision.toString(),
                observedHash = null,
                projectedHash = null,
            ))
        }
        return TasksOrgSyncReport(created, updated, unchanged, deleted, skipped)
    }

    companion object {
        fun sourceId(account: Account, canonicalId: String): String =
            "android/tasks.org/${account.type}/${account.name}/$canonicalId"
    }
}

private fun TasksOrgTask.providerHash(): String = listOf(
    canonicalId, title, notes, dueDateMillis, dueAllDay, startDateMillis,
    startAllDay, completedAtMillis, recurrence, listProviderId, parentProviderId,
).joinToString("\u0000").let { java.security.MessageDigest.getInstance("SHA-256")
    .digest(it.toByteArray()).joinToString("") { byte -> "%02x".format(byte) } }
