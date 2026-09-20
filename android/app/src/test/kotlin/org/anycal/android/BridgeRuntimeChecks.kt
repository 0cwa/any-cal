package org.anycal.android

/** Provider-free bridge selection checks. No endpoint or credential is contacted. */
object BridgeRuntimeChecks {
    @JvmStatic
    fun main(args: Array<String>) = runAll()

    fun runAll() {
        disabledModeIsExplicit()
        invalidEndpointAndCredentialFailClosed()
        nativeUnavailableFailsClosed()
        typedErrorRejectsCredentialText()
    }

    private fun disabledModeIsExplicit() {
        val bridge = RustSyncBridgeFactory.create(BridgeRuntimeConfig("", "", enabled = false))
        check(bridge.syncOnce(emptyCapabilities()) is SyncResult.Unsupported)
        check(bridge.sync(sampleRequest()).error?.code == BridgeErrorCode.TRANSPORT_UNAVAILABLE)
    }

    private fun invalidEndpointAndCredentialFailClosed() {
        check(RustSyncBridgeFactory.create(BridgeRuntimeConfig("file:///tmp/bridge", "handle")) === DisabledRustSyncBridge)
        check(RustSyncBridgeFactory.create(BridgeRuntimeConfig("https://example.invalid", "")) === DisabledRustSyncBridge)
    }

    private fun nativeUnavailableFailsClosed() {
        val bridge = RustSyncBridgeFactory.create(BridgeRuntimeConfig("https://example.invalid", "opaque-handle"))
        check(bridge.syncOnce(emptyCapabilities()) is SyncResult.Unsupported)
    }

    private fun typedErrorRejectsCredentialText() {
        check(runCatching { BridgeError(BridgeErrorCode.INVALID_REQUEST, "authorization token leaked") }.isFailure)
    }

    private fun sampleRequest() = BridgeRequest("space", "org.anycal.account", "com.android.contacts", null, emptyList(), emptyList())

    private fun emptyCapabilities() = ProviderCapabilities(35, false, false, false, false, false, false, false, null)
}
