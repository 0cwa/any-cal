package org.anycal.android.sync

/** Dependency-free decision checks; SQLite/provider behavior remains device-tested. */
object ProjectionStateChecks {
    @JvmStatic
    fun main(args: Array<String>) {
        check(ProjectionDecisions.classify(null, "h", null, false, false) == ProjectionDecision.Create)
        check(ProjectionDecisions.classify(null, null, null, false, true) == ProjectionDecision.Tombstoned)
        val binding = ProjectionBinding("a", "t", "authority", "c", 7, "s", "h1", "h1", "r1", null, null, null)
        check(ProjectionDecisions.classify(binding, "h1", "h1", true, false) == ProjectionDecision.NoOp)
        check(ProjectionDecisions.classify(binding, "h2", "h1", true, false) == ProjectionDecision.Update)
        check(ProjectionDecisions.classify(binding, "h1", "h2", true, false, ProjectionOperationContext("op", "s", "h1", "h3", "r2")) == ProjectionDecision.Conflict)
        check(ProjectionDecisions.classify(binding, "h1", "h3", true, false, ProjectionOperationContext("op", "s", "h1", "h3", "r2")) == ProjectionDecision.ObserverEcho)
        check(ProjectionDecisions.classify(binding.copy(lastOperationId = "op"), "h1", "h3", true, false, ProjectionOperationContext("op", "s", "h1", "h3", "r2")) == ProjectionDecision.Replay)
        check(ProjectionDecisions.classify(binding, "h1", "h1", false, false) == ProjectionDecision.ProviderRecreated)
        check(ProjectionDecisions.classify(binding.copy(tombstoneRevision = "r2"), "h1", "h1", true, false) == ProjectionDecision.Tombstoned)
        check(ProjectionDecisions.classify(binding, "h1", "h1", true, true) == ProjectionDecision.Delete)
    }
}
