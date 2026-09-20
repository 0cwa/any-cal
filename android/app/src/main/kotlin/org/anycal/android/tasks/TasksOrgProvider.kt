package org.anycal.android.tasks

import android.content.ContentResolver
import android.content.ContentValues
import android.database.ContentObserver
import android.database.Cursor
import android.net.Uri
import android.os.Looper
import android.provider.BaseColumns
import java.security.MessageDigest
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** A row read from Tasks.org. The numeric ID is local to this installation. */
data class TasksOrgProviderRow(
    val providerId: Long,
    val title: String,
    val notes: String,
    val dueDateMillis: Long,
    val dueAllDay: Boolean,
    val startDateMillis: Long,
    val startAllDay: Boolean,
    val completedAtMillis: Long,
    val recurrence: String,
    val parentId: Long?,
    val listId: Long?,
    val modifiedAtMillis: Long,
    val readOnly: Boolean,
) {
    fun hash(): String = sha256(listOf(
        providerId, title, notes, dueDateMillis, dueAllDay, startDateMillis,
        startAllDay, completedAtMillis, recurrence, parentId, listId,
        modifiedAtMillis, readOnly,
    ).joinToString("\u0000"))
}

data class TasksOrgProviderSchema(
    val taskColumns: Set<String>,
    val listColumns: Set<String>,
) {
    fun validate(): TasksOrgProviderSchema {
        require(REQUIRED_TASK_COLUMNS.all { it in taskColumns }) { "Tasks.org task schema is incomplete" }
        require(REQUIRED_LIST_COLUMNS.all { it in listColumns }) { "Tasks.org list schema is incomplete" }
        return this
    }

    companion object {
        val REQUIRED_TASK_COLUMNS = setOf(
            BaseColumns._ID, "title", "notes", "due_date", "due_all_day",
            "start_date", "start_all_day", "completed_at", "recurrence",
            "parent_id", "list_id", "modified_at", "is_read_only",
        )
        val REQUIRED_LIST_COLUMNS = setOf(BaseColumns._ID, "title", "access")
    }
}

/** Narrow provider seam. Calls must run off the main thread; Tasks.org documents
 * every operation as a Binder round trip. */
interface TasksOrgProviderGateway {
    fun schema(): TasksOrgProviderSchema
    fun listTasks(limit: Int = 100, offset: Int = 0): List<TasksOrgProviderRow>
    fun insert(task: TasksOrgTask): Long
    fun update(providerId: Long, task: TasksOrgTask): Boolean
    fun read(providerId: Long): TasksOrgProviderRow?
    /** Delete only if the row still has the expected projection hash. */
    fun deleteIfOwned(providerId: Long, expectedHash: String): Boolean
    fun registerObserver(observer: ContentObserver)
    fun unregisterObserver(observer: ContentObserver)
}

class ContentResolverTasksOrgGateway(
    private val resolver: ContentResolver,
    private val adapter: TasksOrgAdapter,
) : TasksOrgProviderGateway {
    private var cachedSchema: TasksOrgProviderSchema? = null

    override fun schema(): TasksOrgProviderSchema {
        checkBackgroundThread()
        check(adapter.probe() is TasksOrgProbeResult.Supported) {
            "Tasks.org provider is unavailable or not allow-listed"
        }
        cachedSchema?.let { return it }
        val taskColumns = queryColumns(TASKS_URI)
        val listColumns = queryColumns(LISTS_URI)
        return TasksOrgProviderSchema(taskColumns, listColumns).validate().also { cachedSchema = it }
    }

    override fun listTasks(limit: Int, offset: Int): List<TasksOrgProviderRow> {
        checkBackgroundThread()
        require(limit in 1..MAX_PAGE) { "Tasks.org page limit is out of bounds" }
        require(offset >= 0) { "Tasks.org page offset must not be negative" }
        val uri = TASKS_URI.buildUpon()
            .appendQueryParameter("limit", limit.toString())
            .appendQueryParameter("offset", offset.toString())
            .build()
        return resolver.query(uri, TASK_PROJECTION, null, null, null)?.use { cursor ->
            buildList {
                while (cursor.moveToNext()) add(cursor.toProviderRow())
            }
        } ?: error("Tasks.org provider returned no cursor")
    }

    override fun insert(task: TasksOrgTask): Long {
        checkBackgroundThread()
        val uri = resolver.insert(TASKS_URI, task.toContentValues())
            ?: error("Tasks.org insert returned no URI")
        return ContentUrisCompat.parseId(uri)
    }

    override fun update(providerId: Long, task: TasksOrgTask): Boolean {
        checkBackgroundThread()
        require(providerId > 0) { "Tasks.org provider ID must be positive" }
        val changed = resolver.update(taskUri(providerId), task.toContentValues(), null, null)
        return changed == 1
    }

    override fun read(providerId: Long): TasksOrgProviderRow? {
        checkBackgroundThread()
        require(providerId > 0) { "Tasks.org provider ID must be positive" }
        return resolver.query(taskUri(providerId), TASK_PROJECTION, null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.toProviderRow() else null
        }
    }

    override fun deleteIfOwned(providerId: Long, expectedHash: String): Boolean {
        checkBackgroundThread()
        val row = read(providerId) ?: return true
        if (row.hash() != expectedHash) return false
        return resolver.delete(taskUri(providerId), null, null) == 1
    }

    override fun registerObserver(observer: ContentObserver) {
        resolver.registerContentObserver(BASE_URI, true, observer)
    }

    override fun unregisterObserver(observer: ContentObserver) {
        resolver.unregisterContentObserver(observer)
    }

    private fun queryColumns(uri: Uri): Set<String> = resolver.query(uri, null, null, null, null)?.use {
        it.columnNames.toSet()
    } ?: error("Tasks.org provider returned no cursor")

    private fun checkBackgroundThread() {
        check(Looper.myLooper() != Looper.getMainLooper()) {
            "Tasks.org provider calls must run off the main thread"
        }
    }

    private fun Cursor.toProviderRow() = TasksOrgProviderRow(
        providerId = getLong(getColumnIndexOrThrow(BaseColumns._ID)),
        title = getString(getColumnIndexOrThrow("title")),
        notes = getString(getColumnIndexOrThrow("notes")),
        dueDateMillis = getLong(getColumnIndexOrThrow("due_date")),
        dueAllDay = getInt(getColumnIndexOrThrow("due_all_day")) != 0,
        startDateMillis = getLong(getColumnIndexOrThrow("start_date")),
        startAllDay = getInt(getColumnIndexOrThrow("start_all_day")) != 0,
        completedAtMillis = getLong(getColumnIndexOrThrow("completed_at")),
        recurrence = getString(getColumnIndexOrThrow("recurrence")),
        parentId = getLong(getColumnIndexOrThrow("parent_id")).takeIf { it != 0L },
        listId = getLong(getColumnIndexOrThrow("list_id")).takeIf { it != 0L },
        modifiedAtMillis = getLong(getColumnIndexOrThrow("modified_at")),
        readOnly = getInt(getColumnIndexOrThrow("is_read_only")) != 0,
    )

    companion object {
        const val MAX_PAGE = 500
        val BASE_URI: Uri = Uri.parse("content://org.tasks.api/v0")
        val TASKS_URI: Uri = Uri.withAppendedPath(BASE_URI, "tasks")
        val LISTS_URI: Uri = Uri.withAppendedPath(BASE_URI, "lists")
        private val TASK_PROJECTION = arrayOf(
            BaseColumns._ID, "title", "notes", "due_date", "due_all_day",
            "start_date", "start_all_day", "completed_at", "recurrence",
            "parent_id", "list_id", "modified_at", "is_read_only",
        )

        private fun taskUri(id: Long) = Uri.withAppendedPath(TASKS_URI, id.toString())
    }
}

private object ContentUrisCompat {
    fun parseId(uri: Uri): Long = uri.lastPathSegment?.toLongOrNull()
        ?: error("Tasks.org provider returned a URI without a numeric ID")
}

private fun TasksOrgTask.toContentValues(): ContentValues = ContentValues().apply {
    require(title.isNotBlank()) { "Tasks.org task title is required" }
    put("title", title)
    put("notes", notes)
    put("due_date", dueDateMillis)
    put("due_all_day", if (dueAllDay) 1 else 0)
    put("start_date", startDateMillis)
    put("start_all_day", if (startAllDay) 1 else 0)
    put("completed_at", completedAtMillis)
    put("recurrence", recurrence)
    listProviderId?.let { put("list_id", it) }
    parentProviderId?.let { put("parent_id", it) }
}

fun TasksOrgProviderRow.toTask(canonicalId: String): TasksOrgTask = TasksOrgTask(
    canonicalId = canonicalId,
    title = title,
    notes = notes,
    dueDateMillis = dueDateMillis,
    dueAllDay = dueAllDay,
    startDateMillis = startDateMillis,
    startAllDay = startAllDay,
    completedAtMillis = completedAtMillis,
    recurrence = recurrence,
    listProviderId = listId,
    parentProviderId = parentId,
)

/** Parse the stable subset of VTODO fields that maps without inventing provider
 * semantics. Unsupported fields stay in the canonical bridge envelope. */
fun org.anycal.android.BridgeResource.toTasksOrgTask(): TasksOrgTask {
    require(kind == "task") { "bridge resource is not a task" }
    val fields = document.fields
    val title = fields["SUMMARY"]?.firstOrNull()?.value
        ?: fields["TITLE"]?.firstOrNull()?.value.orEmpty()
    require(title.isNotBlank()) { "task has no SUMMARY/TITLE" }
    val due = fields["DUE"]?.firstOrNull()?.toTaskDate()
    val start = fields["DTSTART"]?.firstOrNull()?.toTaskDate()
    val completed = fields["COMPLETED"]?.firstOrNull()?.toTaskDate()
    return TasksOrgTask(
        canonicalId = anytypeObjectId,
        title = title,
        notes = fields["DESCRIPTION"]?.firstOrNull()?.value.orEmpty(),
        dueDateMillis = due?.first ?: 0L,
        dueAllDay = due?.second ?: false,
        startDateMillis = start?.first ?: 0L,
        startAllDay = start?.second ?: false,
        completedAtMillis = completed?.first ?: 0L,
        recurrence = fields["RRULE"]?.firstOrNull()?.value.orEmpty(),
    )
}

private fun org.anycal.android.BridgeOccurrence.toTaskDate(): Pair<Long, Boolean> {
    val raw = value
    val allDay = raw.length == 8 && raw.all { it.isDigit() }
    val millis = runCatching {
        if (allDay) LocalDate.parse(raw, DateTimeFormatter.BASIC_ISO_DATE)
            .atStartOfDay(ZoneId.of("UTC")).toInstant().toEpochMilli()
        else Instant.parse(raw).toEpochMilli()
    }.recoverCatching {
        OffsetDateTime.parse(raw).toInstant().toEpochMilli()
    }.recoverCatching {
        LocalDateTime.parse(raw).atZone(ZoneId.of(params["TZID"]?.firstOrNull() ?: "UTC"))
            .toInstant().toEpochMilli()
    }.getOrElse { error("task date is not parseable") }
    return millis to allDay
}

private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
    .digest(value.toByteArray())
    .joinToString("") { "%02x".format(it) }
