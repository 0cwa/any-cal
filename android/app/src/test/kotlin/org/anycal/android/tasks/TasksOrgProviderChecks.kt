package org.anycal.android.tasks

import org.anycal.android.BridgeDocument
import org.anycal.android.BridgeOccurrence
import org.anycal.android.BridgeResource

/** Provider-free checks for the documented Tasks.org v0 mapping and ownership
 * boundary. No ContentResolver or provider writes occur here. */
object TasksOrgProviderChecks {
    @JvmStatic
    fun main(args: Array<String>) = runAll()

    fun runAll() {
        mapsVtodoFieldsWithoutInventingIds()
        rejectsUnsupportedTaskShapes()
        keepsProviderIdsOperationalOnly()
    }

    private fun mapsVtodoFieldsWithoutInventingIds() {
        val resource = BridgeResource(
            collectionId = "personal",
            resourceId = "task:1",
            kind = "task",
            anytypeObjectId = "object-task-1",
            davUid = "uid-task-1",
            document = BridgeDocument(mapOf(
                "SUMMARY" to listOf(BridgeOccurrence("Call contact")),
                "DESCRIPTION" to listOf(BridgeOccurrence("Discuss follow-up")),
                "DUE" to listOf(BridgeOccurrence("20260920T120000Z")),
                "RRULE" to listOf(BridgeOccurrence("RRULE:FREQ=WEEKLY;COUNT=2")),
                "X-CRM-NOTE" to listOf(BridgeOccurrence("preserved in bridge envelope")),
            )),
            revision = 3,
        )
        val task = resource.toTasksOrgTask()
        check(task.canonicalId == "object-task-1")
        check(task.title == "Call contact")
        check(task.notes == "Discuss follow-up")
        check(task.dueDateMillis > 0L && !task.dueAllDay)
        check(task.recurrence == "RRULE:FREQ=WEEKLY;COUNT=2")
        check(task.listProviderId == null && task.parentProviderId == null)
    }

    private fun rejectsUnsupportedTaskShapes() {
        check(runCatching {
            BridgeResource("c", "r", "event", "o", "u", revision = 1).toTasksOrgTask()
        }.isFailure)
        check(runCatching {
            BridgeResource(
                "c", "r", "task", "o", "u",
                BridgeDocument(mapOf("DESCRIPTION" to listOf(BridgeOccurrence("no title")))), 1,
            ).toTasksOrgTask()
        }.isFailure)
        check(runCatching {
            BridgeResource(
                "c", "r", "task", "o", "u",
                BridgeDocument(mapOf(
                    "SUMMARY" to listOf(BridgeOccurrence("bad date")),
                    "DUE" to listOf(BridgeOccurrence("not-a-date")),
                )), 1,
            ).toTasksOrgTask()
        }.isFailure)
    }

    private fun keepsProviderIdsOperationalOnly() {
        val task = TasksOrgTask("canonical", "Task", listProviderId = 42L, parentProviderId = 7L)
        val fields = TasksOrgMapping.columns(task)
        check(fields["list_id"] == 42L)
        check(fields["parent_id"] == 7L)
        check(task.canonicalId != fields["list_id"].toString())
    }
}
