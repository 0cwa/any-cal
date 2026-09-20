# Android native FFI packaging lane

This lane builds the repository's `crates/android-bridge` `cdylib` and stages
the native libraries that the Android module packages. The supported release
ABIs are `arm64-v8a` and `x86_64`; no credentials or provider data are used.

## Preflight

```sh
ANDROID_HOME=/opt/android-sdk \
GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle \
  scripts/android-native-build/preflight.sh
```

The pinned native toolchain is Rust 1.98.1 from the repository
`rust-toolchain.toml` and `cargo-ndk` 4.1.2. Install the latter with
`cargo install cargo-ndk --version 4.1.2 --locked` before running the lane.
The preflight reports JDK, Gradle, Cargo, `cargo-ndk`, installed NDK versions,
and whether any `Cargo.toml` declares `crate-type = ["cdylib"]`. It exits 3
when native source is not ready.

## ABI build

```sh
ANDROID_HOME=/opt/android-sdk \
ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.2.12479018 \
NATIVE_CRATE_DIR="$PWD/crates/android-bridge" \
  scripts/android-native-build/build-abis.sh
```

The script writes generated libraries below
`android/app/build/generated/jniLibs`. Set `CARGO_TARGET_DIR` to a directory
under `./tmp` when a local host has a small system temporary filesystem.

## APK load smoke gate

```sh
APK_PATH=android/app/build/outputs/apk/debug/app-debug.apk \
  scripts/android-native-build/load-smoke.sh
```

This gate checks ABI library presence without installing the APK or touching
ContactsContract, CalendarContract, Tasks.org, or Anytype. The future device
smoke test should install only into a disposable API-35 emulator, launch a
credential-free native bridge health test, and capture `logcat` without
provider CRUD.

## Sparse VM recipe

```sh
msb create ubuntu:24.04 -n any-cal-android-native-build \
  -c 4 --max-cpus 4 -m 4G --max-memory 8G \
  --root-disk 32G --thp madvise
```

Install JDK 17, Gradle 8.10.2, Android command-line tools, the pinned NDK,
Rust 1.98.1, and cargo-ndk 4.1.2 inside that VM only. Stop and remove the VM after
evidence capture.
