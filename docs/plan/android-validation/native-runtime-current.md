# Android native runtime revalidation

Date: 2026-09-20

## Disposition

This document retains the earlier failed TCG revalidation for historical
traceability. The later authoritative receipts supersede its runtime
conclusion: `host-runtime-current.md` proves the API-35 host-KVM provider/JNI
probe, and `inprocess-smoke-current.md` proves the credential-free JNI calls
inside the installed app process. `bridge-integration-current.md` is the
authoritative current ABI/APK packaging receipt. Android TLS trust, real
AccountManager lifecycle, and production Anytype reconciliation remain
separate gates.

No personal Flatpak, Android profile, provider account, Anytype credential, or
host credential was mounted or used.

## Environment and isolation

Validation ran in the disposable microsandbox
`any-cal-android-native-build-20260919b` (Ubuntu 24.04, four vCPUs, 4 GiB
initial/8 GiB maximum memory, 32 GiB managed sparse root disk). Source was
copied selectively: `Cargo.toml`, `Cargo.lock`, `crates/`, `android/`, and
`scripts/android-native-build/`. The host repository and hidden credential
directories were not mounted.

The build VM was stopped after evidence capture. A separate
`any-cal-native-runtime-revalidate-20260920` VM was created but not used for
the build and was removed after the run.

## Evidence

### Follow-up revalidation (2026-09-20)

The current checked-in `build-abis.sh` was rerun after its `cargo ndk`
invocation repair:

```text
cargo 1.98.1
cargo-ndk 4.1.2
cdylib_crate=1
android-native-build preflight: PASS
android-native-build ABI compilation: PASS
```

The APK was rebuilt with `:app:assembleDebug :app:testDebugUnitTest`; Gradle
reported `BUILD SUCCESSFUL` (41 actionable tasks).

The API-35 Google APIs x86_64 image was acquired in the disposable VM after a
bounded download (the archive completed and unpacked). A snapshot was then
booted in a fresh disposable VM with 8 GiB memory. The emulator process started
and ADB initially reported a device, but the framework never completed boot:

```text
adb shell getprop sys.boot_completed -> empty
adb install -r app-debug.apk ->
  cmd: Can't find service: package
adb devices -> emulator-5554 offline (after the final boot attempt)
```

The first 4-GiB VM was independently confirmed to be OOM-killed by its guest
kernel while running the emulator:

```text
invoked oom-killer
oom-kill ... task=qemu-system-x86
Out of memory: Killed process ... qemu-system-x86
```

The retry VM had enough memory and did not show an OOM record, but its clean
TCG boot still exited before `package` and `activity` services became
available. The redacted emulator log ended with the emulator shutdown path;
there was no APK install, `System.loadLibrary`, or JNI return-value evidence.

The disposable VMs and API-35 snapshot were stopped/removed after evidence
capture. The temporary probe APK source was removed. No personal Flatpak,
Android profile, provider account, Anytype credential, or host credential was
mounted or used.

### Native source and ABI builds

The current source contains:

```text
crates/android-bridge/Cargo.toml: crate-type = ["cdylib"]
android/app/build.gradle.kts: abiFilters = ["arm64-v8a", "x86_64"]
android/app/src/main/kotlin/org/anycal/android/NativeRustBridge.kt
```

The checked-in script was attempted with:

```sh
cd /opt/work
ANDROID_HOME=/opt/android-sdk \
ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.2.12479018 \
NATIVE_CRATE_DIR=/opt/work/crates/android-bridge \
NATIVE_ABIS=arm64-v8a,x86_64 \
scripts/android-native-build/build-abis.sh
```

The repaired script now uses the supported `cargo ndk` subcommand and passed.
The equivalent individual commands are:

```sh
cargo ndk -t arm64-v8a build --release \
  --manifest-path /opt/work/crates/android-bridge/Cargo.toml
cargo ndk -t x86_64 build --release \
  --manifest-path /opt/work/crates/android-bridge/Cargo.toml
```

Both passed. Resulting artifacts:

```text
aarch64-linux-android  libany_cal_android_bridge.so  789392 bytes
x86_64-linux-android   libany_cal_android_bridge.so  764800 bytes
```

Checksums from the disposable VM:

```text
e1b2264663bf24546348fefc16c349b24d7d19e72bef216357a90a8746ed0e81  aarch64-linux-android/release/libany_cal_android_bridge.so
82bfddccb7e6eeebf6510310e35fed07cb003ed4ab76ec673ac08f9b5f0be18b  x86_64-linux-android/release/libany_cal_android_bridge.so
```

`readelf` confirmed ELF64 shared objects for AArch64 and x86-64. `nm -D`
confirmed these exported JNI entrypoints in both libraries:

```text
Java_org_anycal_android_NativeRustBridge_nativeSchemaVersion
Java_org_anycal_android_NativeRustBridge_nativeHealth
Java_org_anycal_android_NativeRustBridge_nativeBridgeJson
```

### APK packaging

The libraries were staged under the Gradle generated JNI directory and built
with:

```sh
cd /opt/work/android
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon --offline \
  :app:assembleDebug :app:testDebugUnitTest
```

Result: **BUILD SUCCESSFUL**, 41 actionable tasks, including
`:app:testDebugUnitTest`.

The APK load smoke check passed:

```text
native-library=arm64-v8a:present
native-library=x86_64:present
android-native-load smoke preflight: PASS
```

APK metadata:

```text
package: name='org.anycal.android' versionCode='1'
versionName='0.1.0-foundation' compileSdkVersion='35' targetSdkVersion='35'
minSdkVersion='26'
```

APK SHA-256:

```text
ae8eec9375bdd1b187d75646a7d59e571dc0333a9062e46298d852d23ecbd561
```

### Historical runtime gate

The following failed attempt is retained as historical evidence only; it is
not the current runtime disposition.

The earlier VM did not have an emulator binary or system image. A later
disposable install was completed with:

```sh
sdkmanager --install emulator \
  "system-images;android-35;google_apis;x86_64"
```

The image was unpacked successfully, but the API-35 emulator could not finish
framework startup under TCG. The package manager service was absent when APK
installation was attempted. Consequently, there is no valid evidence yet for
APK installation, `System.loadLibrary`, or calls to `nativeSchemaVersion`,
`nativeHealth`, and `nativeBridgeJson` on Android.

## Unresolved gaps

1. A disposable Android runtime with working package/activity services is still
   required for actual APK install and JNI calls. The API-35 image is now
   available, but the nested TCG emulator did not complete framework startup;
   runtime success is not claimed.
2. The native bridge currently returns a credential-free `NOT_LINKED` response
   for bridge JSON requests; this revalidation proves the native loading seam
   only, not Anytype transport or provider projection behavior.

## Next action

Use a host-supported or hardware-accelerated disposable API-35 Android runtime
(or a prebuilt CI emulator runner) rather than nested TCG, then run this
minimum runtime gate:

1. install the debug APK with `adb install -r`;
2. invoke a credential-free Android instrumentation/shell probe that calls
   `NativeRustBridge.negotiate()`, `health()`, and one malformed plus one valid
   `requestJson` request;
3. capture package/version, return values, and redacted `logcat`;
4. stop and remove the VM/emulator after evidence capture.
