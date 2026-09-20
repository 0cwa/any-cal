# Android native bridge integration validation

Date: 2026-09-20

## Scope and isolation

This receipt covers the Android native bridge/service foundation only. The
run used the disposable host-KVM microsandbox
`any-cal-android-bridge-20260920` (Ubuntu 24.04, 4 vCPUs, 4 GiB initial / 8
GiB maximum memory, 32 GiB sparse root disk), with JDK 17, Gradle 8.10.2,
Android SDK API 35, Build-Tools 34.0.0, NDK 27.2.12479018, Rust 1.98.1, and
`cargo-ndk` 4.1.2.

No personal Flatpak/profile was mounted. No Anytype credential, endpoint,
ContentResolver/provider row, AccountManager account, or Tasks.org data was
used. The emulator was disposable and was removed after the run.

## Build and package gates

The native ABI script compiled both declared targets and staged the libraries:

```text
android-native-build ABI compilation: PASS
aarch64-linux-android/release/libany_cal_android_bridge.so 789480 bytes
x86_64-linux-android/release/libany_cal_android_bridge.so 764888 bytes
```

The complete Android harness then passed:

```text
BUILD SUCCESSFUL in 43s
88 actionable tasks: 88 executed
Tasks.org contract checks passed: no provider access or CRUD performed
apk-entry=lib/arm64-v8a/libany_cal_android_bridge.so:present
apk-entry=lib/x86_64/libany_cal_android_bridge.so:present
apk-metadata=package:org.anycal.android minSdk:26 targetSdk:35
android-release-apk assertion: PASS
android-build assemble/unit/verifier: PASS
```

Checksums captured in the VM:

```text
arm64-v8a  ade011b7e16b7192d24556e3fa29bff9c9ff8023924abd92214fb6f2fd3b49a9
x86_64     7dec8abdf9e028489019f4a25e0fe1d1b033a170bb9aed72ca88ee6913f4be1d
debug APK  196c0f14be726a6650963a4e4366a6f0f04b5758b8cc9c045c36eb6830dc12c5
release APK e6e80f78f419775a5bbf103b929bd0c5da6be94d2095ddd3b1a8f9b890a02a68
```

Gradle emitted the existing non-fatal `Unable to strip ...
libany_cal_android_bridge.so` diagnostic; the unstripped libraries were
packaged and the explicit APK entry assertion passed.

## Direct provider-free checks

`BridgeRuntimeChecks` was invoked directly and exited 0. It covered explicit
disabled mode, malformed endpoint/credential fail-closed behavior, unavailable
native readiness, and rejection of credential text in typed errors.

The following checks also exited 0. Android framework shims were temporary
in-memory JVM shims only; they did not contact Android services:

```text
AccountLifecycleChecks: PASS
ContactsProjectionChecks: PASS
ContactsSyncCallbackChecks: PASS
ContactsReconciliationChecks: PASS
CalendarProjectionAcceptanceTest: PASS
CalendarSyncCallbackAcceptanceTest: PASS
CalendarBidirectionalReconcilerAcceptanceTest: PASS
CalendarContractStoreUriAcceptanceTest: PASS
ProjectionStateChecks: PASS
```

The Gradle `:app:testDebugUnitTest` task passed, but its report contained only
binary result files and no XML-discovered test cases. The direct entrypoint
results above are therefore the authoritative evidence for these acceptance
objects.

## API-35 emulator package gate

A fresh API-35 Google APIs x86_64 emulator booted successfully:

```text
sdk=35
boot=1
```

The debug APK installed successfully without account setup or provider access:

```text
Success
primaryCpuAbi=x86_64
versionCode=1 minSdk=26 targetSdk=35
versionName=0.1.0-foundation
```

This proves API-35 installation and package/native-ABI metadata. It does not
claim live Anytype synchronization, provider CRUD, or Android account
lifecycle completion; those remain separate gates.

## Remaining boundary

The next runtime gate is a small credential-free Android probe that calls
`NativeRustBridge.available()`, `negotiate()`, and `health()` inside the
installed app process. The current receipt proves those calls through the
provider-free JVM contract and proves packaging/install, but does not claim an
in-process JNI invocation on the API-35 emulator.
