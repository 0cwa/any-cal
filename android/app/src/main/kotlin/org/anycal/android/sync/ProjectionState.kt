package org.anycal.android.sync

data class ProjectionBinding(
    val accountName: String,
    val accountType: String,
    val authority: String,
    val canonicalId: String,
    val providerRowId: Long?,
    val sourceId: String,
    val projectedHash: String?,
    val observedHash: String?,
    val canonicalRevision: String?,
    val tombstoneRevision: String?,
    val lastOperationId: String?,
    val lastOperationPostHash: String?,
) {
    fun validate(): ProjectionBinding {
        require(accountName.isNotBlank() && accountType.isNotBlank()) { "account scope is required" }
        require(authority.isNotBlank() && canonicalId.isNotBlank() && sourceId.isNotBlank()) { "projection identity is required" }
        return this
    }
}

data class ProjectionOperationContext(
    val operationId: String,
    val sourceId: String,
    val preHash: String?,
    val postHash: String?,
    val canonicalRevision: String?,
)

enum class ProjectionDecision {
    Create,
    Update,
    NoOp,
    Delete,
    Replay,
    ObserverEcho,
    ProviderRecreated,
    Conflict,
    Tombstoned,
}

object ProjectionDecisions {
    fun classify(
        binding: ProjectionBinding?,
        canonicalHash: String?,
        providerHash: String?,
        providerRowPresent: Boolean,
        deleteRequested: Boolean,
        operation: ProjectionOperationContext? = null,
    ): ProjectionDecision {
        if (binding == null) return if (deleteRequested) ProjectionDecision.Tombstoned else ProjectionDecision.Create
        if (binding.tombstoneRevision != null && !deleteRequested) return ProjectionDecision.Tombstoned
        if (deleteRequested) return ProjectionDecision.Delete
        if (operation != null && binding.lastOperationId == operation.operationId && providerHash == operation.postHash) {
            return ProjectionDecision.Replay
        }
        if (operation != null && providerHash == operation.postHash) return ProjectionDecision.ObserverEcho
        if (binding.lastOperationPostHash != null && providerHash == binding.lastOperationPostHash) return ProjectionDecision.ObserverEcho
        if (binding.providerRowId != null && !providerRowPresent) return ProjectionDecision.ProviderRecreated
        if (canonicalHash == binding.projectedHash && providerHash == binding.observedHash) return ProjectionDecision.NoOp
        if (operation != null && providerHash != null && operation.preHash != null && providerHash != operation.preHash) return ProjectionDecision.Conflict
        if (!providerRowPresent) return ProjectionDecision.Create
        return ProjectionDecision.Update
    }
}
