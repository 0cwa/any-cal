# Android native TLS validation

Date: 2026-09-21

## Source gate

The Android bridge uses `rustls-platform-verifier` 0.7.0 and its Android
support component 0.1.1. The bridge now:

- aligns its JNI dependency with the verifier's JNI 0.22 API;
- exposes `nativeInitializeVerifier(Context)` and calls
  `rustls_platform_verifier::android::init_with_env` with the process JVM and
  application context;
- resolves the verifier support AAR from Cargo metadata in the Android
  dependency repositories; and
- refuses an HTTPS request from Kotlin until verifier initialization succeeds.

There is no certificate bypass, hostname bypass, HTTP fallback for HTTPS, or
credential-bearing diagnostic path.

## Evidence

The following host checks passed using only workspace sources and disposable
build output under `./tmp`:

```text
env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/home/linuxbrew/.linuxbrew/bin:/usr/bin:/bin \
  CARGO_TARGET_DIR=./tmp/android-bridge-host \
  cargo check -p any-cal-android-bridge --offline
  -> pass

env -u LD_PRELOAD CARGO_TARGET_DIR=./tmp/android-bridge-test \
  cargo test -p any-cal-android-bridge --offline --locked
  -> source test suite includes endpoint policy, bounded response framing,
     schema/error redaction, and platform verifier configuration construction
```

The Android-target check could not be completed in the current host because
the pinned Android NDK clang toolchain is absent. The required verifier support
crate was downloaded to Cargo's user cache during the check, but no NDK or APK
was produced by this receipt.

## Runtime boundary

No Android trusted public endpoint or trusted disposable CA endpoint was
exercised after this change. Existing API-35 JNI smoke evidence proves native
library loading and credential-free bridge calls only. Therefore this receipt
does not claim Android certificate-chain acceptance, proxy behavior, or live
Anytype HTTPS. The remaining gate is a disposable API-35 build with the
pinned NDK and an HTTPS endpoint whose certificate is trusted by the Android
system verifier; the run must also cover wrong-host and unknown-CA rejection.
