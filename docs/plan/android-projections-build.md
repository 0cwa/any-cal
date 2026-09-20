# Android projection-stage build validation

Date: 2026-09-19

## Environment

Validation used a fresh disposable microsandbox VM:

- Name: `any-cal-android-build-20260919`
- Image: Ubuntu 24.04
- Root disk: 32 GiB managed sparse ext4
- Resources: 4 vCPUs, 4 GiB boot memory, 8 GiB maximum
- JDK: OpenJDK 17.0.20
- Gradle: 8.10.2
- Android SDK: API 35 platform, platform-tools, Build-Tools 34

No host mounts, emulator, provider data, Anytype credentials, or live Anytype
operations were used.

## Command

The current Android module was copied into the disposable VM and built with:

```sh
ANDROID_HOME=/opt/android-sdk \
ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon :app:assembleDebug
```

## First complete compiler diagnostics

The authenticator fix compiled successfully. The projection-stage build now
stops in the Calendar slice:

```text
e: .../calendar/CalendarContractStore.kt:75:14 Overload resolution ambiguity between candidates:
fun <T> Iterable<T>.sumOf(selector: (T) -> Int): Int
fun <T> Iterable<T>.sumOf(selector: (T) -> Long): Long
e: .../calendar/CalendarContractStore.kt:82:14 Overload resolution ambiguity between candidates:
fun <T> Iterable<T>.sumOf(selector: (T) -> Int): Int
fun <T> Iterable<T>.sumOf(selector: (T) -> Long): Long

Execution failed for task ':app:compileDebugKotlin'.
BUILD FAILED in 2m 20s
```

The SDK tooling also emitted the known non-fatal warning that the installed
SDK metadata is version 4 while this command-line tools release understands
through version 3.

## Disposition

The authenticator blocker is cleared. The next smallest product fix is to make
the two `sumOf` selectors in `CalendarContractStore.kt` explicitly `Int` or
`Long`, according to the intended return type, then rerun the same command.
This validation lane did not edit product files. The disposable VM was stopped
and removed after capturing the diagnostics.

## Repeat after Calendar `sumOf` fix

Date: 2026-09-19

A fresh sparse VM (`any-cal-android-build-20260919-r2`, 32 GiB root disk,
4 vCPUs, 4 GiB memory) was provisioned with the same JDK 17, Gradle 8.10.2,
API-35 SDK, and Build-Tools 34 recipe. The current Android source was copied
into the VM without modification.

Build command:

```sh
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon :app:assembleDebug
```

Result: `BUILD SUCCESSFUL` in 2m31s; all 33 actionable tasks completed.
The SDK XML version-4 warning remains non-fatal.

The probe’s local deterministic gate also passed on the host:

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

No emulator or provider data was used. The fresh VM was stopped and removed
after the successful build and evidence capture.

## Final projection acceptance gate

Date: 2026-09-19

In a new 32 GiB sparse VM (`any-cal-android-accept-20260919`), the current
Android app passed the final build and compatible unit-test gates:

```sh
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon :app:assembleDebug
```

Result: `BUILD SUCCESSFUL in 2m17s`, 33 actionable tasks.

```sh
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon :app:testDebugUnitTest
```

Result: `BUILD SUCCESSFUL in 15s`, 22 actionable tasks. The task ran without
an emulator and performed no provider writes. The dependency-free Tasks.org
verifier also passed on the host:

```text
Tasks.org contract checks passed: no provider access or CRUD performed
```

The known SDK XML version-4 warning and Gradle deprecation warning were
non-fatal. The VM was stopped and removed after evidence capture.
