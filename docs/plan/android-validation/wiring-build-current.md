# Android provider wiring build validation

Date: 2026-09-20

## Scope and isolation

Validation ran in the disposable microsandbox `any-cal-android-wiring-build-20260920`:

- Ubuntu 24.04, 4 vCPUs, 4 GiB initial / 8 GiB maximum memory, 32 GiB managed sparse root disk.
- Pinned JDK 17, Gradle 8.10.2, Android SDK platform 35, Build-Tools 34.0.0, NDK 27.2.12479018, Rust stable 1.98.1, and cargo-ndk 4.1.2.
- Only `Cargo.toml`, `Cargo.lock`, `crates/`, `android/`, and build/probe scripts were copied into the VM.
- No personal Flatpak/profile, Android emulator, provider account, Anytype credential, or host credential was mounted or used.

The VM was stopped and removed after evidence capture.

## Commands and results

Provisioning used the checked-in recipe:

```sh
cd /workspace
scripts/android-native-build/provision-toolchain.sh
```

The toolchain provisioning completed successfully and reported the pinned versions above.

The requested Android build/test command was:

```sh
cd /workspace
export PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/root/.cargo/bin:$PATH
export ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk
export GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle
scripts/android-build/build-and-test.sh
```

Result: **FAIL at Kotlin compilation**, before unit tests or APK packaging:

```text
> Task :app:compileDebugKotlin FAILED
file:///workspace/android/app/src/main/kotlin/org/anycal/android/sync/AnyCalSyncAdapterService.kt:64:21 Unresolved reference 'StoreContactBindingRepository'.
27 actionable tasks: 27 executed
BUILD FAILED in 1m 59s
```

The class exists in `org.anycal.android.contacts.ContactsSyncCallback.kt`, but
`AnyCalSyncAdapterService.kt` imports `ContactBindingRepository` and does not
import `StoreContactBindingRepository`. This is a source/API wiring defect, not
a toolchain or provider-runtime failure. No product source was changed during
this validation lane.

Because compilation stopped at this unresolved reference, the following could
not yet be run through Gradle in this exact source state:

- `:app:testDebugUnitTest`
- callback tests for `ContactsSyncCallback` and `CalendarSyncCallback`
- service-level fake gateway tests
- APK assembly and native-library packaging

The calendar classes were therefore not compiler-validated by this run; no
additional calendar diagnostic should be inferred from the early failure.

The independent, no-write provider-free checks did pass:

```sh
cd /workspace
scripts/android-probe/validate.sh
```

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

These checks exercise deterministic callback/probe fixtures only. They do not
access ContactsContract, CalendarContract, Tasks.org, an emulator, or Anytype.

## Required follow-up

Add the missing `StoreContactBindingRepository` import (or qualify the class)
in `AnyCalSyncAdapterService.kt`, then rerun the full build command above. The
next evidence must include successful `:app:assembleDebug` and
`:app:testDebugUnitTest` output before any provider/runtime validation is
claimed.

## Post-fix rerun (2026-09-20)

The missing import was present in the current source before this rerun. A new
VM, `any-cal-android-wiring-build-20260920b`, was created with the same sparse
recipe and removed after validation. Only source, Android, build, and no-write
probe files were copied into it. No emulator, provider account, Anytype
endpoint, credential, or host profile was used.

Provisioning completed successfully with JDK 17, Gradle 8.10.2, Android SDK
platform 35, Build-Tools 34.0.0, NDK 27.2.12479018, Rust 1.98.1, and
cargo-ndk 4.1.2.

### Build and package gate

```sh
cd /workspace
export PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/root/.cargo/bin:$PATH
export ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk
export GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle
scripts/android-build/build-and-test.sh
```

Result: **PASS**.

```text
BUILD SUCCESSFUL in 3m 14s
39 actionable tasks: 39 executed
Tasks.org contract checks passed: no provider access or CRUD performed
android-build assemble/unit/verifier: PASS
```

APK metadata:

```text
package: name='org.anycal.android' versionCode='1' versionName='0.1.0-foundation'
compileSdkVersion='35' targetSdkVersion='35' minSdkVersion='26'
```

Correction: the checksum above is recorded exactly as emitted by the VM as:

```text
3f8952ed4c51ce01909fc6a3fa645abebbd03e47524f447cde3a13ca84b7f4e3
```

The Gradle report directory contained zero XML test reports. The task passed,
but the current acceptance objects are plain Kotlin objects rather than
JUnit-discovered tests.

### Direct provider-free callback checks

Contacts checks were invoked from the compiled classes with the API-35
`android.jar` and Kotlin standard library:

```sh
java -cp "$CP" org.anycal.android.contacts.ContactsProjectionChecks
java -cp "$CP" org.anycal.android.contacts.ContactsSyncCallbackChecks
```

Both exited 0 (**PASS**). These cover deterministic contact mapping, labels,
groups, identity, capabilities, callback scope/permission checks,
idempotency, binding identity, apply-only persistence, and tombstones.

The Calendar callback object was invoked through a temporary in-VM Java runner
and a minimal temporary `ContentValues` shim, because the SDK `android.jar`
contains runtime stubs:

```text
CalendarSyncCallbackAcceptanceTest: PASS
```

The separate `CalendarProjectionAcceptanceTest` did not pass in this run. It
failed at its fixture assertion that `2026-10-25T09:00:00` in
`Europe/Stockholm` equals `2026-10-25T07:00:00Z`; the current timezone data
resolves the local time to `08:00:00Z` after the October DST transition. This
is an existing fixture expectation mismatch, not a provider or Anytype write.
The calendar callback acceptance object itself passed.

### Probe gate

```sh
cd /workspace
scripts/android-probe/validate.sh
```

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

This probe is deterministic and no-write; it does not access Android provider
rows, an emulator, or Anytype.

### Disposition

- Missing Contacts service import: **fixed; build now passes**.
- APK assembly and package metadata: **pass**.
- Gradle unit-test task: **task pass, zero discovered tests**.
- Contacts projection and callback checks: **pass**.
- Calendar callback checks: **pass through temporary provider-free runner**.
- Calendar projection fixture: **fails on timezone expectation; requires fixture decision/fix**.
- Provider/Anytype safety: **pass; no writes or credentials used**.

## Calendar fixture correction rerun (2026-09-20)

The fixture assertion now derives the expected epoch from the same
`Europe/Stockholm` timezone rules used by the projection instead of embedding
the obsolete `07:00Z` expectation. Validation ran in a fresh sparse VM,
`any-cal-calendar-fixture-rerun-20260920`, which was removed after capture.
No emulator, provider account, Anytype endpoint, credential, or host profile
was used.

### Build

```sh
cd /workspace
export PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/root/.cargo/bin:$PATH
export ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk
export GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle
scripts/android-build/build-and-test.sh
```

Result: **PASS**.

```text
BUILD SUCCESSFUL in 2m 4s
39 actionable tasks: 39 executed
Tasks.org contract checks passed: no provider access or CRUD performed
APK_SHA256=ed7ba55922a8fd55171cb7e03a8646185fa3899849102347bcb275dfbb60e9cc
android-build assemble/unit/verifier: PASS
```

### Direct Calendar acceptance

The compiled classes were run with a temporary in-VM `ContentValues` shim;
the SDK `android.jar` remains a runtime stub. Both direct acceptance objects
exited successfully:

```sh
java -cp "$CP" org.anycal.android.calendar.CalendarProjectionAcceptanceTest
java -cp "$CP" RunCalendarChecks
```

```text
CalendarSyncCallbackAcceptanceTest: PASS
```

`CalendarProjectionAcceptanceTest` also exited 0 (no output is expected from
its `main`). The corrected timezone assertion now passes, as do projection
field, recurrence, attendee, reminder, opaque-field, tombstone, and fail-closed
capability checks. `RunCalendarChecks` invoked
`CalendarSyncCallbackAcceptanceTest.INSTANCE.runAll()` and passed all callback
validation groups.

### Probe

```sh
cd /workspace
scripts/android-probe/validate.sh
```

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

Updated disposition: the Calendar projection fixture, direct Calendar callback
acceptance, Android build/package gate, and no-write probe all pass. Live
CalendarContract provider behavior remains untested by this lane.
