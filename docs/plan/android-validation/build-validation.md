# Reproducible Android build/probe validation

Date: 2026-09-19

## Disposable execution

The harness was run in microsandbox `any-cal-android-build-validation-20260919`:

- Ubuntu 24.04, 4 vCPUs, 4 GiB memory, 32 GiB managed sparse root disk.
- JDK 17.0.20.
- Gradle 8.10.2.
- Android SDK platform 35 and Build-Tools 34.0.0.
- No host mounts, personal Flatpak/profile, provider writes, or Anytype access.

The VM was removed after evidence capture.

## Harness command and evidence

```sh
ANDROID_HOME=/opt/android-sdk \
ANDROID_SDK_ROOT=/opt/android-sdk \
GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle \
  scripts/android-build/build-and-test.sh
```

The harness reported:

```text
android-build preflight: PASS
BUILD SUCCESSFUL in 2m 15s
39 actionable tasks: 39 executed
Tasks.org contract checks passed: no provider access or CRUD performed
APK_SHA256=01c0f851cc00fd5d97957d2004c299cb6e18bd1cb1430721f06eb3df52993189
package: name='org.anycal.android' versionCode='1' versionName='0.1.0-foundation' platformBuildVersionName='15' platformBuildVersionCode='35' compileSdkVersion='35' compileSdkVersionCodename='15'
android-build assemble/unit/verifier: PASS
```

The dependency-free probe gate was also run inside the VM:

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

This build VM deliberately did not contain an emulator/system image, so no
live provider capability probe or provider data access occurred in this run.
The API-35 live capability probe remains covered by the earlier redacted
evidence in `docs/plan/android-toolchain-validation.md`.

## Reproduction assets

- `scripts/android-build/preflight.sh`: JDK/Gradle/SDK gate.
- `scripts/android-build/build-and-test.sh`: assemble, unit tests, verifier,
  APK checksum, and safe package metadata.
- `scripts/android-build/README.md`: sparse VM recipe and capability-probe
  command.
- `scripts/android-build/ci-template.yml`: no-secret CI template.
- `scripts/android-probe/`: no-write provider capability probe and offline
  deterministic test.
