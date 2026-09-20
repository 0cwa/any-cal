package org.anycal.android

import org.json.JSONArray
import org.json.JSONObject

const val BRIDGE_SCHEMA_VERSION: Int = 1

data class BridgeCheckpoint(val cursor: String, val revision: Long)

data class BridgeOccurrence(
    val value: String,
    val params: Map<String, List<String>> = emptyMap(),
)

data class BridgeDocument(
    val fields: Map<String, List<BridgeOccurrence>> = emptyMap(),
)

/** A validated, provider-neutral resource payload. Provider row IDs never cross
 * this boundary as canonical identity. */
data class BridgeResource(
    val collectionId: String,
    val resourceId: String,
    val kind: String,
    val anytypeObjectId: String,
    val davUid: String,
    val document: BridgeDocument = BridgeDocument(),
    val revision: Long,
) {
    fun validate(): BridgeResource {
        require(collectionId.isNotBlank() && resourceId.isNotBlank() && kind.isNotBlank()) {
            "bridge resource identity is required"
        }
        require(anytypeObjectId.isNotBlank() && davUid.isNotBlank()) {
            "bridge resource canonical identity is required"
        }
        require(revision >= 0L) { "bridge resource revision must not be negative" }
        document.fields.forEach { (name, values) ->
            require(name.isNotBlank()) { "bridge property name must not be blank" }
            values.forEach { occurrence ->
                require(occurrence.value.indexOf('\u0000') < 0) { "bridge property contains NUL" }
                occurrence.params.forEach { (key, params) ->
                    require(key.isNotBlank() && params.all { it.indexOf('\u0000') < 0 }) {
                        "bridge property parameter is invalid"
                    }
                }
            }
        }
        return this
    }
}

data class BridgeTombstone(
    val resourceId: String,
    val canonicalId: String,
    val revision: Long,
    val collectionId: String? = null,
)

data class BridgeRequest(
    val accountName: String,
    val accountType: String,
    val authority: String,
    val checkpoint: BridgeCheckpoint?,
    val resources: List<String>,
    val tombstones: List<BridgeTombstone>,
    val schemaVersion: Int = BRIDGE_SCHEMA_VERSION,
    /** Full payloads are supplied for writes. The ID list remains for stable
     * compatibility with the original callback and test seam. */
    val resourcePayloads: List<BridgeResource> = emptyList(),
) {
    fun validate(): BridgeRequest {
        require(schemaVersion == BRIDGE_SCHEMA_VERSION) { "unsupported bridge schema version: $schemaVersion" }
        require(accountName.isNotBlank() && accountType.isNotBlank() && authority.isNotBlank()) {
            "bridge account and authority are required"
        }
        require(resources.distinct().size == resources.size && resources.all { it.isNotBlank() }) {
            "bridge resource IDs must be unique and non-blank"
        }
        resourcePayloads.forEach { it.validate() }
        require(resourcePayloads.map { it.anytypeObjectId }.distinct().size == resourcePayloads.size) {
            "bridge resource payload IDs must be unique"
        }
        require(resourcePayloads.all { it.anytypeObjectId in resources }) {
            "bridge resource payload is outside the requested scope"
        }
        require(tombstones.all { it.canonicalId.isNotBlank() && it.resourceId.isNotBlank() && it.revision > 0L }) {
            "bridge tombstones must contain a canonical ID and positive revision"
        }
        return this
    }
}

enum class BridgeDecision { NOOP, UPSERT, ARCHIVE, CONFLICT, UNSUPPORTED }
enum class BridgeErrorCode { UNSUPPORTED_VERSION, INVALID_REQUEST, NOT_LINKED, PERMISSION_DENIED, TRANSPORT_UNAVAILABLE, CONFLICT }

data class BridgeDecisionResult(
    val resourceId: String,
    val decision: BridgeDecision,
    val reason: String? = null,
)

data class BridgeError(val code: BridgeErrorCode, val message: String) {
    init {
        val lower = message.lowercase()
        require(listOf("token", "api_key", "authorization", "password", "secret").none { lower.contains(it) }) {
            "bridge error must not contain credentials"
        }
    }
}

data class BridgeResponse(
    val checkpoint: BridgeCheckpoint?,
    val decisions: List<BridgeDecisionResult>,
    val error: BridgeError?,
    val schemaVersion: Int = BRIDGE_SCHEMA_VERSION,
    val resources: List<BridgeResource> = emptyList(),
    val tombstones: List<BridgeTombstone> = emptyList(),
) {
    fun validate(): BridgeResponse {
        require(schemaVersion == BRIDGE_SCHEMA_VERSION) { "unsupported bridge schema version: $schemaVersion" }
        resources.forEach { it.validate() }
        require(resources.map { it.anytypeObjectId }.distinct().size == resources.size) {
            "bridge response resource IDs must be unique"
        }
        require(tombstones.all { it.canonicalId.isNotBlank() && it.resourceId.isNotBlank() && it.revision > 0L }) {
            "bridge response tombstones must contain a canonical ID and positive revision"
        }
        return this
    }
}

/** Versioned provider-to-gateway seam. The configured implementation delegates
 * transport to the Rust JNI bridge; the explicit no-op remains available for
 * disabled/unconfigured accounts and deterministic tests. */
interface RustSyncBridge {
    fun syncOnce(capabilities: ProviderCapabilities): SyncResult

    /** Deterministic DTO boundary; no transport or authentication is implied. */
    fun sync(request: BridgeRequest): BridgeResponse =
        BridgeResponse(
            checkpoint = request.validate().checkpoint,
            decisions = emptyList(),
            error = BridgeError(BridgeErrorCode.NOT_LINKED, "Rust sync bridge is not linked"),
        )
}

sealed interface SyncResult {
    data object NotLinked : SyncResult
    data object Ready : SyncResult
    data class Unsupported(val reason: String) : SyncResult
}

object NoOpRustSyncBridge : RustSyncBridge {
    override fun syncOnce(capabilities: ProviderCapabilities): SyncResult =
        SyncResult.NotLinked
}

internal fun BridgeResource.toJson(): JSONObject = JSONObject().apply {
    put("collection_id", collectionId)
    put("resource_id", resourceId)
    put("kind", kind)
    put("anytype_object_id", anytypeObjectId)
    put("dav_uid", davUid)
    put("revision", revision)
    put("document", JSONObject().apply {
        put("version", 1)
        put("content", JSONObject().apply {
            put("fields", JSONObject().apply {
                document.fields.toSortedMap().forEach { (name, occurrences) ->
                    put(name, JSONArray().apply {
                        occurrences.forEach { occurrence ->
                            put(JSONObject().apply {
                                put("value", occurrence.value)
                                if (occurrence.params.isNotEmpty()) {
                                    put("params", JSONObject().apply {
                                        occurrence.params.toSortedMap().forEach { (key, values) ->
                                            put(key, JSONArray(values))
                                        }
                                    })
                                }
                            })
                        }
                    })
                }
            })
        })
    })
}

internal fun BridgeTombstone.toJson(): JSONObject = JSONObject().apply {
    put("resource_id", resourceId)
    put("canonical_id", canonicalId)
    put("revision", revision)
    collectionId?.let { put("collection_id", it) }
}
