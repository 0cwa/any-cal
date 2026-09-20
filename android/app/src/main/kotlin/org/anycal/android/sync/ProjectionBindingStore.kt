package org.anycal.android.sync

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import org.anycal.android.BridgeCheckpoint

/** Durable account/authority-scoped projection identity and loop metadata. */
class ProjectionBindingStore(context: Context) : SQLiteOpenHelper(
    context,
    DATABASE_NAME,
    null,
    DATABASE_VERSION,
) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE $TABLE (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_name TEXT NOT NULL,
                account_type TEXT NOT NULL,
                authority TEXT NOT NULL,
                canonical_id TEXT NOT NULL,
                provider_row_id INTEGER,
                source_id TEXT NOT NULL,
                projected_hash TEXT,
                observed_hash TEXT,
                canonical_revision TEXT,
                tombstone_revision TEXT,
                last_operation_id TEXT,
                last_operation_post_hash TEXT,
                UNIQUE(account_name, account_type, authority, canonical_id),
                UNIQUE(account_name, account_type, authority, source_id)
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX $TABLE" + "_source ON $TABLE(account_name, account_type, authority, source_id)")
        createCheckpointTable(db)
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) {
            createCheckpointTable(db)
        }
    }

    fun get(accountName: String, accountType: String, authority: String, canonicalId: String): ProjectionBinding? {
        validateScope(accountName, accountType, authority, canonicalId)
        readableDatabase.query(
            TABLE,
            COLUMNS,
            "account_name=? AND account_type=? AND authority=? AND canonical_id=?",
            arrayOf(accountName, accountType, authority, canonicalId),
            null,
            null,
            null,
        ).use { cursor -> return if (cursor.moveToFirst()) read(cursor) else null }
    }

    fun upsert(binding: ProjectionBinding) {
        binding.validate()
        val values = ContentValues().apply {
            put("account_name", binding.accountName)
            put("account_type", binding.accountType)
            put("authority", binding.authority)
            put("canonical_id", binding.canonicalId)
            binding.providerRowId?.let { put("provider_row_id", it) } ?: putNull("provider_row_id")
            put("source_id", binding.sourceId)
            putNullable("projected_hash", binding.projectedHash)
            putNullable("observed_hash", binding.observedHash)
            putNullable("canonical_revision", binding.canonicalRevision)
            putNullable("tombstone_revision", binding.tombstoneRevision)
            putNullable("last_operation_id", binding.lastOperationId)
            putNullable("last_operation_post_hash", binding.lastOperationPostHash)
        }
        writableDatabase.insertWithOnConflict(TABLE, null, values, SQLiteDatabase.CONFLICT_REPLACE).also {
            check(it != -1L) { "failed to persist projection binding" }
        }
    }

    fun markTombstone(binding: ProjectionBinding, revision: String) {
        upsert(binding.copy(tombstoneRevision = revision))
    }

    fun clearTombstone(binding: ProjectionBinding) {
        upsert(binding.copy(tombstoneRevision = null))
    }

    fun checkpoint(accountName: String, accountType: String, authority: String, generation: String): BridgeCheckpoint? {
        validateScope(accountName, accountType, authority, generation)
        readableDatabase.query(
            CHECKPOINT_TABLE,
            CHECKPOINT_COLUMNS,
            "account_name=? AND account_type=? AND authority=? AND generation=?",
            arrayOf(accountName, accountType, authority, generation),
            null, null, null,
        ).use { cursor ->
            if (!cursor.moveToFirst()) return null
            return BridgeCheckpoint(cursor.text("cursor"), cursor.getLong(cursor.getColumnIndexOrThrow("revision")))
        }
    }

    fun saveCheckpoint(accountName: String, accountType: String, authority: String, generation: String, checkpoint: BridgeCheckpoint) {
        validateScope(accountName, accountType, authority, generation)
        require(checkpoint.cursor.isNotBlank() && checkpoint.revision >= 0L) { "checkpoint is invalid" }
        val values = ContentValues().apply {
            put("account_name", accountName)
            put("account_type", accountType)
            put("authority", authority)
            put("generation", generation)
            put("cursor", checkpoint.cursor)
            put("revision", checkpoint.revision)
        }
        check(writableDatabase.insertWithOnConflict(CHECKPOINT_TABLE, null, values, SQLiteDatabase.CONFLICT_REPLACE) != -1L) {
            "failed to persist sync checkpoint"
        }
    }

    fun clearCheckpoint(accountName: String, accountType: String, authority: String) {
        validateScope(accountName, accountType, authority, "clear")
        writableDatabase.delete(CHECKPOINT_TABLE, "account_name=? AND account_type=? AND authority=?", arrayOf(accountName, accountType, authority))
    }

    fun bindings(accountName: String, accountType: String, authority: String): List<ProjectionBinding> {
        validateScope(accountName, accountType, authority, "list")
        readableDatabase.query(
            TABLE,
            COLUMNS,
            "account_name=? AND account_type=? AND authority=?",
            arrayOf(accountName, accountType, authority),
            null, null, "source_id ASC",
        ).use { cursor ->
            val result = mutableListOf<ProjectionBinding>()
            while (cursor.moveToNext()) result += read(cursor)
            return result
        }
    }

    private fun read(cursor: android.database.Cursor) = ProjectionBinding(
        accountName = cursor.text("account_name"),
        accountType = cursor.text("account_type"),
        authority = cursor.text("authority"),
        canonicalId = cursor.text("canonical_id"),
        providerRowId = cursor.longOrNull("provider_row_id"),
        sourceId = cursor.text("source_id"),
        projectedHash = cursor.textOrNull("projected_hash"),
        observedHash = cursor.textOrNull("observed_hash"),
        canonicalRevision = cursor.textOrNull("canonical_revision"),
        tombstoneRevision = cursor.textOrNull("tombstone_revision"),
        lastOperationId = cursor.textOrNull("last_operation_id"),
        lastOperationPostHash = cursor.textOrNull("last_operation_post_hash"),
    )

    private fun validateScope(accountName: String, accountType: String, authority: String, canonicalId: String) {
        require(accountName.isNotBlank() && accountType.isNotBlank() && authority.isNotBlank() && canonicalId.isNotBlank()) {
            "projection scope is required"
        }
    }

    private fun ContentValues.putNullable(key: String, value: String?) {
        if (value == null) putNull(key) else put(key, value)
    }

    companion object {
        private const val DATABASE_NAME = "anycal-projection-state.db"
        private const val DATABASE_VERSION = 2
        private const val TABLE = "projection_bindings"
        private const val CHECKPOINT_TABLE = "sync_checkpoints"
        private val COLUMNS = arrayOf(
            "account_name", "account_type", "authority", "canonical_id", "provider_row_id",
            "source_id", "projected_hash", "observed_hash", "canonical_revision",
            "tombstone_revision", "last_operation_id", "last_operation_post_hash",
        )

        private val CHECKPOINT_COLUMNS = arrayOf("cursor", "revision")

        private fun createCheckpointTable(db: SQLiteDatabase) {
            db.execSQL(
                """
                CREATE TABLE IF NOT EXISTS $CHECKPOINT_TABLE (
                    account_name TEXT NOT NULL,
                    account_type TEXT NOT NULL,
                    authority TEXT NOT NULL,
                    generation TEXT NOT NULL,
                    cursor TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    PRIMARY KEY(account_name, account_type, authority)
                )
                """.trimIndent(),
            )
        }
    }
}

private fun android.database.Cursor.text(name: String): String = getString(getColumnIndexOrThrow(name))
private fun android.database.Cursor.textOrNull(name: String): String? {
    val index = getColumnIndexOrThrow(name)
    return if (isNull(index)) null else getString(index)
}
private fun android.database.Cursor.longOrNull(name: String): Long? {
    val index = getColumnIndexOrThrow(name)
    return if (isNull(index)) null else getLong(index)
}
