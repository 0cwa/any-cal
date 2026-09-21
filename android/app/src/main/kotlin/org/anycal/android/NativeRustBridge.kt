package org.anycal.android

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** JNI loader for the versioned Android-to-gateway bridge. */
object NativeRustBridge {
    private val loadError: Throwable? = runCatching { System.loadLibrary("any_cal_android_bridge") }.exceptionOrNull()
    @Volatile private var verifierInitialized = false

    fun available(): Boolean = loadError == null

    fun negotiate(): Boolean = loaded { nativeSchemaVersion() == BRIDGE_SCHEMA_VERSION }

    fun health(): Boolean = loaded { nativeHealth() == 1 }

    fun ready(): Boolean = available() && runCatching { negotiate() && health() }.getOrDefault(false)

    /** Binds rustls-platform-verifier to the process JVM and Android trust store. */
    fun initializeVerifier(context: Context): Boolean {
        if (!available()) return false
        if (verifierInitialized) return true
        return synchronized(this) {
            if (verifierInitialized) {
                true
            } else {
                runCatching {
                    nativeInitializeVerifier(context.applicationContext) == 1
                }.getOrDefault(false).also { verifierInitialized = it }
            }
        }
    }

    fun requestJson(request: BridgeRequest, endpoint: String = "", credential: String = ""): String {
        request.validate()
        require(credential.indexOfAny(charArrayOf('\r', '\n', '\u0000')) < 0) {
            "bridge credential contains invalid characters"
        }
        check(!endpoint.startsWith("https://", ignoreCase = true) || verifierInitialized) {
            "native TLS verifier is not initialized"
        }
        return loaded { nativeBridgeJson(request.toJson(), endpoint, credential) }
    }

    private fun <T> loaded(block: () -> T): T = check(loadError == null) {
        "Rust bridge native library is unavailable: ${loadError?.message ?: "unknown error"}"
    }.let { block() }

    @JvmStatic private external fun nativeSchemaVersion(): Int
    @JvmStatic private external fun nativeHealth(): Int
    @JvmStatic private external fun nativeInitializeVerifier(context: Context): Int
    @JvmStatic private external fun nativeBridgeJson(request: String, endpoint: String, credential: String): String

    private fun BridgeRequest.toJson(): String = JSONObject().apply {
        put("schema_version", schemaVersion)
        put("account_name", accountName)
        put("account_type", accountType)
        put("authority", authority)
        checkpoint?.let {
            put("checkpoint", JSONObject().put("cursor", it.cursor).put("revision", it.revision))
        }
        val payloads = if (resourcePayloads.isNotEmpty()) {
            JSONArray().apply { resourcePayloads.forEach { put(it.toJson()) } }
        } else {
            JSONArray()
        }
        put("resources", payloads)
        // The ID list is a bounded Android-side scope hint. The Rust/core
        // request validates the full envelope objects above.
        put("resource_ids", JSONArray(resources))
        put("tombstones", JSONArray().apply {
            tombstones.forEach { put(it.toJson()) }
        })
    }.toString()
}

private fun JSONObject.readStringArray(key: String): List<String> =
    optJSONArray(key)?.let { values -> buildList { for (index in 0 until values.length()) add(values.getString(index)) } }
        ?: emptyList()

private fun parseBridgeOccurrence(value: JSONObject): BridgeOccurrence {
    val params = buildMap {
        val raw = value.optJSONObject("params") ?: return@buildMap
        raw.keys().asSequence().toList().sorted().forEach { key ->
            put(key, raw.readStringArray(key))
        }
    }
    return BridgeOccurrence(value.getString("value"), params)
}

internal fun parseBridgeResource(value: JSONObject): BridgeResource {
    val document = value.optJSONObject("document")?.optJSONObject("content")?.optJSONObject("fields")?.let { fields ->
        BridgeDocument(buildMap {
            fields.keys().asSequence().toList().sorted().forEach { name ->
                val occurrences = fields.getJSONArray(name)
                put(name, buildList {
                    for (index in 0 until occurrences.length()) add(parseBridgeOccurrence(occurrences.getJSONObject(index)))
                })
            }
        })
    } ?: BridgeDocument()
    return BridgeResource(
        collectionId = value.getString("collection_id"),
        resourceId = value.getString("resource_id"),
        kind = value.getString("kind"),
        anytypeObjectId = value.getString("anytype_object_id"),
        davUid = value.getString("dav_uid"),
        document = document,
        revision = value.getLong("revision"),
    ).validate()
}

internal fun parseBridgeTombstone(value: JSONObject) = BridgeTombstone(
    resourceId = value.getString("resource_id"),
    canonicalId = value.getString("canonical_id"),
    revision = value.getLong("revision"),
    collectionId = value.optString("collection_id").takeIf { it.isNotBlank() },
)
