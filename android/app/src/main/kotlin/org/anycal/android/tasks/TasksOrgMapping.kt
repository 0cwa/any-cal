package org.anycal.android.tasks

/** Provider-neutral task fields; IDs remain canonical Anytype IDs, not provider row IDs. */
data class TasksOrgTask(
    val canonicalId: String,
    val title: String,
    val notes: String = "",
    val dueDateMillis: Long = 0,
    val dueAllDay: Boolean = false,
    val startDateMillis: Long = 0,
    val startAllDay: Boolean = false,
    val completedAtMillis: Long = 0,
    val recurrence: String = "",
    val listProviderId: Long? = null,
    val parentProviderId: Long? = null,
)

/** Deterministic column projection only; a provider adapter must gate writes first. */
object TasksOrgMapping {
    fun columns(task: TasksOrgTask): Map<String, Any> = linkedMapOf(
        "title" to task.title,
        "notes" to task.notes,
        "due_date" to task.dueDateMillis,
        "due_all_day" to if (task.dueAllDay) 1 else 0,
        "start_date" to task.startDateMillis,
        "start_all_day" to if (task.startAllDay) 1 else 0,
        "completed_at" to task.completedAtMillis,
        "recurrence" to task.recurrence,
    ) + listOfNotNull(
        task.listProviderId?.let { "list_id" to it },
        task.parentProviderId?.let { "parent_id" to it },
    )
}
