# Android native FFI packaging validation

Date: 2026-09-20

## Disposition

The native source, ABI compilation, and APK packaging gates have partial
evidence. A disposable toolchain VM manually built `arm64-v8a` and `x86_64`
JNI libraries and packaged them into a debug APK. The checked-in
`build-abis.sh` still fails before compilation because the installed
`cargo-ndk` wrapper requires the `cargo ndk` invocation form. APK installation,
`System.loadLibrary`, and credential-free JNI calls remain unverified.

## Reproducible lane assets

- `scripts/android-native-build/preflight.sh` checks JDK, Gradle, Cargo,
  cargo-ndk, installed NDK versions, and `cdylib` source presence.
- `scripts/android-native-build/build-abis.sh` defines the ABI build seam for
  `arm64-v8a`, `armeabi-v7a`, `x86_64`, and `x86` once a bridge crate exists.
- `scripts/android-native-build/load-smoke.sh` checks that an APK contains one
  `.so` per declared ABI without installing or launching it.
- `scripts/android-native-build/README.md` documents the pinned-toolchain and
  sparse-VM recipe.

## Disposable preflight evidence

VM: `any-cal-android-native-preflight-20260919`, Ubuntu 24.04, 2 vCPUs,
2 GiB memory, 8 GiB managed sparse root disk. JDK 17.0.20 was installed.

Command:

```sh
cd /opt/work
scripts/android-native-build/preflight.sh
```

Redacted output:

```text
openjdk version "17.0.20"
javac 17.0.20
cargo=missing
cargo-ndk=missing
ANDROID_HOME=missing
ndk=missing
gradle=missing
cdylib_crate=0
native-artifact-source=missing
android-native-build preflight: environment inspected; native source is not ready
PREFLIGHT_EXIT=3
```

The VM was stopped and removed after preflight. No provider data, APK install,
Anytype access, or credentials were used. The later disposable build receipt
is recorded in `docs/plan/android-validation/native-runtime-current.md`.

## Minimum prerequisite to continue

The build-script repair and reproducible ABI/package lane are now complete.
The next gate is to use `load-smoke.sh` and a disposable API-35 instrumentation
test. Until APK installation and
JNI calls are observed, ABI/package evidence alone is not a native runtime
acceptance signal.

## Native toolchain provisioning handoff

Date: 2026-09-19

A fresh sparse VM (`any-cal-android-native-toolchain-20260919`, Ubuntu 24.04,
4 vCPUs, 4 GiB memory, 32 GiB managed root disk) was provisioned using
`scripts/android-native-build/provision-toolchain.sh`. Public toolchain
downloads completed successfully:

```text
ANDROID_SDK_ROOT=/opt/android-sdk
ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.2.12479018
openjdk version "17.0.20"
Gradle 8.10.2
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
cargo-ndk 4.1.2
aarch64-linux-android
x86_64-linux-android
```

Installed Android components include command-line tools, platform-tools,
platform 35, Build-Tools 34.0.0, and NDK 27.2.12479018. This environment
produced the manual ABI/package evidence in the current runtime receipt. The
VM remains disposable and is to be stopped/removed after handoff; no provider
or Anytype data was accessed.
