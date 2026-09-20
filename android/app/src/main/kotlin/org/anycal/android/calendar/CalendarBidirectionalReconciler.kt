package org.anycal.android.calendar

/** A row read from CalendarContract after provider-side change detection. */
data class CalendarObservedRecord(
    val sourceId: String,
    val envelope: CalendarEnvelope?,
    val projectionHash: String?,
    val operationId: String?,
    val deleted: Boolean = false,
)

data class CalendarBinding(
    val sourceId: String,
    val lastProjectedHash: String?,
    val lastOperationId: String?,
    val accountGeneration: Long,
)

data class CalendarReconcileCheckpoint(val token: String?, val accountGeneration: Long)

sealed interface CalendarOutboundChange {
    data class Upsert(val envelope: CalendarEnvelope, val sourceId: String) : CalendarOutboundChange
    data class Tombstone(val sourceId: String) : CalendarOutboundChange
}

sealed interface CalendarReconcileResult {
    data class Applied(val changes: List<CalendarOutboundChange>, val checkpoint: CalendarReconcileCheckpoint) : CalendarReconcileResult
    data class Rejected(val reason: String) : CalendarReconcileResult
}

/** Narrow transport boundary; the implementation may enqueue into Rust/Anytype later. */
fun interface CalendarOutboundSink {
    fun emit(changes: List<CalendarOutboundChange>, checkpoint: CalendarReconcileCheckpoint): Result<Unit>
}

/** ContentObserver-facing source contract; implementations must query rows, not trust observer payloads. */
fun interface CalendarProviderChangeSource {
    fun read(token: String?, limit: Int): Result<CalendarProviderBatch>
}

data class CalendarProviderBatch(val rows: List<CalendarObservedRecord>, val nextToken: String?)

/**
 * Provider-neutral reconciliation. The provider reader owns ContentResolver queries and
 * supplies rows in bounded batches; this class owns echo suppression and restart semantics.
 */
class CalendarBidirectionalReconciler(private val batchSize: Int = 100) {
    init { require(batchSize > 0) }

    fun reconcile(
        rows: List<CalendarObservedRecord>,
        bindings: List<CalendarBinding>,
        checkpoint: CalendarReconcileCheckpoint,
        nextToken: String?,
        accountGeneration: Long,
    ): CalendarReconcileResult {
        if (checkpoint.accountGeneration != accountGeneration) return CalendarReconcileResult.Rejected("account generation changed")
        if (rows.size > batchSize) return CalendarReconcileResult.Rejected("provider batch exceeds limit")
        val bindingMap = bindings.associateBy { it.sourceId }
        if (bindingMap.size != bindings.size) return CalendarReconcileResult.Rejected("duplicate binding")
        val changes = rows.sortedBy { it.sourceId }.mapNotNull { row ->
            val binding = bindingMap[row.sourceId]
            if (binding != null && binding.accountGeneration == accountGeneration &&
                binding.lastProjectedHash == row.projectionHash && binding.lastOperationId == row.operationId) return@mapNotNull null
            if (row.deleted || row.envelope == null) CalendarOutboundChange.Tombstone(row.sourceId)
            else CalendarOutboundChange.Upsert(row.envelope, row.sourceId)
        }
        return CalendarReconcileResult.Applied(changes, CalendarReconcileCheckpoint(nextToken, accountGeneration))
    }
}
