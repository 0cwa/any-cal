# Android account lifecycle build validation

Date: 2026-09-20

## Scope and isolation

The current AccountLifecycle/Authenticator implementation was validated in a
fresh sparse host-KVM microsandbox named
`any-cal-android-account-build-20260920`:

- Ubuntu 24.04
- 4 vCPUs (maximum 4)
- 4 GiB initial / 8 GiB maximum memory
- 32 GiB managed sparse root disk
- JDK 17.0.20, Gradle 8.10.2, Android SDK platform 35, Build-Tools 34.0.0
- No personal Flatpak or Android profile
- No Anytype endpoint, credential, provider account, provider row, or live
  provider write

Only the Android source/tests, generated native library input, and no-write
validation scripts were copied into the VM. A temporary JVM-only Android
framework shim was used solely to invoke plain Kotlin acceptance entrypoints;
the shim was not part of the Android module or APK.

## Build and Gradle test task

Command run in the VM:

```sh
cd /root/android
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon --console=plain \
  :app:assembleDebug :app:testDebugUnitTest
```

Result: **PASS**.

```text
BUILD SUCCESSFUL in 9s
41 actionable tasks: 41 up-to-date
```

The debug APK was produced successfully:

```text
SHA-256: 51141727422556fad37a4187558af9fe7dbfd0182b3a6bf8e6be7bb1cca78b55
package: name='org.anycal.android' versionCode='1' versionName='0.1.0-foundation'
platformBuildVersionCode='35' compileSdkVersion='35' targetSdkVersion='35'
minSdkVersion='26'
```

The Gradle task is not sufficient evidence by itself: its report contains
binary result files but zero XML test reports. The current acceptance classes
are plain Kotlin entrypoints rather than JUnit-discovered tests.

## Explicit provider-free checks

Each entrypoint was invoked directly after compilation. The temporary shim
provided only the Android framework methods needed by these provider-free
checks; no `ContentResolver`, account manager, Contacts provider, Calendar
provider, Tasks.org provider, or Anytype service was contacted.

```text
AccountLifecycleChecks.main()                              EXIT=0 PASS
ContactsProjectionChecks.main()                            EXIT=0 PASS
ContactsSyncCallbackChecks.main()                          EXIT=0 PASS
ContactsReconciliationChecks.main()                        EXIT=0 PASS
CalendarProjectionAcceptanceTest.main()                    EXIT=0 PASS
CalendarSyncCallbackAcceptanceTest.INSTANCE.runAll()       EXIT=0 PASS
CalendarBidirectionalReconcilerAcceptanceTest.INSTANCE.runAll() EXIT=0 PASS
CalendarContractStoreUriAcceptanceTest.INSTANCE.runAll()   EXIT=0 PASS
```

The lifecycle checks covered account creation, duplicate account generation,
credential rotation through the fake store, cleanup-before-removal,
re-addition with a new generation, fail-closed token/permission behavior,
and secret-safe cleanup diagnostics.

The Contacts checks covered mapping, callback scope and permissions,
idempotency, binding identity, tombstones, and checkpoint reconciliation. The
Calendar checks covered projection fields, recurrence/timezone handling,
attendees/reminders, callback validation, bidirectional echo suppression,
tombstones, generation/batch rejection, URI construction, and checkpoint
recovery.

## Disposition

- Account lifecycle compilation: **pass**.
- Authenticator/account foundation packaging: **pass** through the debug APK
  build.
- Explicit AccountLifecycle checks: **pass**.
- Existing Contacts provider-free checks: **pass**.
- Existing Calendar provider-free checks: **pass**.
- Gradle test discovery: **zero tests discovered**; direct entrypoints remain
  required until these checks become JUnit/instrumentation tests.
- Live ContactsContract/CalendarContract behavior: **not claimed** by this
  lane; it requires a dedicated Android framework/emulator provider test.
- Anytype transport or credentials: **not used**.

The microsandbox was stopped and removed after evidence capture.
