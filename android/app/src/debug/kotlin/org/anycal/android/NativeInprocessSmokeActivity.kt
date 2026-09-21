package org.anycal.android

import android.app.Activity
import android.os.Bundle
import android.util.Log

/** Debug-only credential-free in-process JNI smoke harness. */
class NativeInprocessSmokeActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        Thread {
            try {
                val available = NativeRustBridge.available()
                val negotiate = NativeRustBridge.negotiate()
                val health = NativeRustBridge.health()
                val verifier = NativeRustBridge.initializeVerifier(this)
                Log.i(TAG, "stage=readiness available=$available negotiate=$negotiate health=$health verifier_initialized=$verifier")
                check(available && negotiate && health && verifier) { "native readiness failed" }
                val valid = BridgeRequest(
                    accountName = "space-a",
                    accountType = "org.anycal",
                    authority = "com.android.contacts",
                    checkpoint = null,
                    resources = emptyList(),
                    tombstones = emptyList(),
                )
                val response = NativeRustBridge.requestJson(valid)
                val notLinked = response.contains("NOT_LINKED", ignoreCase = true)
                Log.i(TAG, "stage=valid_response length=${response.length} not_linked=$notLinked")
                check(notLinked) { "unexpected credential-free response" }
                val malformed = valid.copy(accountName = "")
                val malformedError = runCatching { NativeRustBridge.requestJson(malformed) }.exceptionOrNull()
                Log.i(TAG, "stage=malformed error=${malformedError?.javaClass?.simpleName} redacted=true")
                check(malformedError is IllegalArgumentException)
                Log.i(TAG, "available=$available negotiate=$negotiate health=$health verifier_initialized=$verifier valid_not_linked=true malformed=redacted")
                setResult(RESULT_OK)
            } catch (error: Throwable) {
                Log.e(TAG, "in-process native smoke failed: ${error::class.java.simpleName}")
                setResult(RESULT_CANCELED)
            } finally {
                finish()
            }
        }.start()
    }

    private companion object { const val TAG = "AnyCalNativeSmoke" }
}
