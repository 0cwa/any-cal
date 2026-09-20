package org.anycal.android

import android.net.Uri
import org.json.JSONArray

data class BridgeRuntimeConfig(
    val endpoint: String,
    val credentialHandle: String,
    val enabled: Boolean = true,
) {
    fun validate(): BridgeRuntimeConfig {
        if (!enabled) return this
        val parsed = Uri.parse(endpoint)
        require(parsed.scheme == "https" || parsed.scheme == "http") { "bridge endpoint must use http or https" }
        require(!parsed.host.isNullOrBlank()) { "bridge endpoint host is required" }
        require(credentialHandle.isNotBlank()) { "bridge credential handle is required" }
        val loopback = parsed.host == "localhost" || parsed.host == "127.0.0.1" || parsed.host == "[::1]" || parsed.host == "::1"
        require(parsed.scheme == "https" || loopback) {
            "bridge HTTP is permitted only for loopback development endpoints"
        }
        return this
    }
}

class NativeRustSyncBridge(
    private val config: BridgeRuntimeConfig,
    private val credentialProvider: () -> String? = { null },
) : RustSyncBridge {
    init { config.validate() }

    override fun syncOnce(capabilities: ProviderCapabilities): SyncResult {
        if (!config.enabled) return SyncResult.Unsupported("bridge is disabled")
        if (!NativeRustBridge.ready()) return SyncResult.Unsupported("native bridge is unavailable or unhealthy")
        return if (credentialProvider()?.isNullOrBlank() == false) SyncResult.Ready
        else SyncResult.Unsupported("bridge credential is unavailable")
    }

    override fun sync(request: BridgeRequest): BridgeResponse {
        config.validate()
        request.validate()
        if (!config.enabled) return BridgeResponse(request.checkpoint, emptyList(), BridgeError(BridgeErrorCode.TRANSPORT_UNAVAILABLE, "bridge is disabled"))
        if (!NativeRustBridge.ready()) return BridgeResponse(request.checkpoint, emptyList(), BridgeError(BridgeErrorCode.TRANSPORT_UNAVAILABLE, "native bridge is unavailable or unhealthy"))
        val credential = credentialProvider()?.takeIf { it.isNotBlank() }
            ?: return BridgeResponse(request.checkpoint, emptyList(), BridgeError(BridgeErrorCode.TRANSPORT_UNAVAILABLE, "bridge credential is unavailable"))
        return runCatching { parseBridgeResponse(NativeRustBridge.requestJson(request, config.endpoint, credential)) }
            .getOrElse { BridgeResponse(request.checkpoint, emptyList(), BridgeError(BridgeErrorCode.TRANSPORT_UNAVAILABLE, "native bridge exchange failed")) }
    }
}

private fun parseBridgeResponse(json: String): BridgeResponse {
    val root = org.json.JSONObject(json)
    val checkpoint = root.optJSONObject("checkpoint")?.let {
        BridgeCheckpoint(it.getString("cursor"), it.getLong("revision"))
    }
    val decisions = buildList {
        val values = root.optJSONArray("decisions") ?: JSONArray()
        for (index in 0 until values.length()) {
            val value = values.getJSONObject(index)
            add(BridgeDecisionResult(
                value.getString("resource_id"),
                BridgeDecision.valueOf(value.getString("decision").uppercase()),
                value.optString("reason").takeIf { it.isNotEmpty() },
            ))
        }
    }
    val resources = buildList {
        val values = root.optJSONArray("resources") ?: JSONArray()
        for (index in 0 until values.length()) add(parseBridgeResource(values.getJSONObject(index)))
    }
    val tombstones = buildList {
        val values = root.optJSONArray("tombstones") ?: JSONArray()
        for (index in 0 until values.length()) add(parseBridgeTombstone(values.getJSONObject(index)))
    }
    val error = root.optJSONObject("error")?.let {
        BridgeError(BridgeErrorCode.valueOf(it.getString("code").uppercase()), it.getString("message"))
    }
    return BridgeResponse(checkpoint, decisions, error, root.getInt("schema_version"), resources, tombstones).validate()
}

object RustSyncBridgeFactory {
    fun create(config: BridgeRuntimeConfig, credentialProvider: () -> String? = { null }): RustSyncBridge = runCatching {
        config.validate()
        if (!config.enabled) DisabledRustSyncBridge else NativeRustSyncBridge(config, credentialProvider)
    }.getOrElse { DisabledRustSyncBridge }
}

object DisabledRustSyncBridge : RustSyncBridge {
    override fun syncOnce(capabilities: ProviderCapabilities): SyncResult = SyncResult.Unsupported("bridge configuration is unavailable")
    override fun sync(request: BridgeRequest): BridgeResponse = BridgeResponse(
        request.validate().checkpoint, emptyList(), BridgeError(BridgeErrorCode.TRANSPORT_UNAVAILABLE, "bridge configuration is unavailable"),
    )
}
