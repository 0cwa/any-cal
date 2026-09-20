# Isolated Android build/probe validation

This lane validates the Android foundation/projection package without a
personal Flatpak, Android profile, provider data, Anytype credentials, or
provider CRUD. It is suitable for a disposable Ubuntu 24.04 microsandbox or a
clean CI runner.

## Preflight and build

Provision JDK 17, Gradle 8.10.2, Android command-line tools, API 35,
Build-Tools 34, NDK 27.2.12479018, Rust, and `cargo-ndk`. Then run from the
repository root:

```sh
ANDROID_HOME=/opt/android-sdk \
GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle \
  scripts/android-build/build-and-test.sh
```

The script builds the `crates/android-bridge` `cdylib` for `arm64-v8a` and
`x86_64`, runs `:app:assembleDebug`, `:app:assembleRelease`,
`:app:testDebugUnitTest`, the dependency-free Tasks.org contract verifier, and
emits a SHA-256 plus safe APK badging metadata. It never installs the APK.

## Capability probe

When a dedicated API-35+ emulator is available, run the existing no-write
probe with a dedicated ADB server:

```sh
ANDROID_ADB_SERVER_PORT=5041 adb -P 5041 start-server
ANDROID_ADB_SERVER_PORT=5041 python3 scripts/android-probe/probe.py \
  --adb "$ANDROID_HOME/platform-tools/adb" --serial emulator-5556 \
  > capability.json
```

The probe distinguishes unavailable ADB/device state from absent Contacts,
Calendar, or Tasks.org authorities and does not enumerate account names or
provider rows.

## Disposable VM recipe

The validation VM used by this lane is sparse and has no host mounts:

```sh
msb create ubuntu:24.04 -n any-cal-android-validation \
  -c 4 --max-cpus 4 -m 4G --max-memory 8G \
  --root-disk 32G --thp madvise
```

Install only the declared toolchain inside the VM, copy the repository Android
module and probe script into the VM, run the commands above, then:

```sh
msb stop any-cal-android-validation
msb remove any-cal-android-validation
```

Do not mount personal home directories or reuse a personal Android/Flatpak
profile. No secrets are required by this lane.
