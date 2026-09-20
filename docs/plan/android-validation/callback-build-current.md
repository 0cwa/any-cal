# Android callback build validation

Date: 2026-09-20

## Scope and isolation

Validation covered the current `ContactsSyncCallback` and
`CalendarSyncCallback` slices. It ran in the fresh sparse microsandbox
`any-cal-callback-build-20260920` (Ubuntu 24.04, 4 vCPUs, 4/8 GiB memory,
32 GiB logical root). Only `android/` and the Android build/probe scripts were
copied into the VM. No host mounts, personal Flatpak/profile, provider
accounts, Anytype credentials, provider writes, emulator, or live service were
used.

The VM was stopped and removed after evidence capture.

## Toolchain

- OpenJDK 17.0.20 (JDK and `javac`)
- Gradle 8.10.2
- Android SDK platform 35
- Android Build-Tools 34.0.0
- Android NDK 27.2.12479018 and Rust/cargo-ndk were provisioned, although
  this callback-only gate did not need an ABI build

The SDK manager emitted its existing XML-version warning (the command-line
tools understand SDK XML through version 3 while one metadata file is version
4); it did not affect the build.

## Build evidence

Command:

```sh
cd /workspace/android
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon \
  :app:assembleDebug :app:testDebugUnitTest
```

Result: **BUILD SUCCESSFUL**, 39 actionable tasks, including
`:app:compileDebugKotlin` and `:app:testDebugUnitTest`.

APK:

```text
e8c4c1a5745a75b4e331e29154d00b2d27671ef2a9915b6984cf1381aa99825e
```

Package metadata: `org.anycal.android`, version `0.1.0-foundation`,
`compileSdkVersion=35`, `targetSdkVersion=35`, `minSdkVersion=26`.

The Gradle report contains **0 discovered tests, 0 failures, 0 ignored**.
The callback acceptance objects are plain Kotlin checks rather than JUnit
tests, so a successful Gradle task does not prove their assertions ran.

The existing dependency-free Tasks.org contract verifier also passed:

```text
Tasks.org contract checks passed: no provider access or CRUD performed
```

## Explicit callback checks

The compiled provider-free checks were invoked through a temporary Java
harness inside the VM. Contacts scope, permission, binding-identity, and
ordering/idempotency checks passed. The Contacts tombstone check failed:

```text
ContactsSyncCallbackChecks.persistsOnlyAfterApplyAndHandlesTombstone
ContactsSyncCallbackChecks.kt:89
IllegalStateException: Check failed
```

The failure is a real callback defect, not a harness-only issue. In the apply
path, a tombstone has no entry in `byId`, so `contact` is null; the binding is
saved with `tombstoneRevision = if (deleted) contact.canonicalRevision else
null`, which cannot persist the tombstone revision. The callback should carry
the tombstone revision independently of the optional payload contact before
the binding save. This validation lane intentionally did not edit product
code.

The separate Contacts mapper check also failed at its existing email-order
assertion (`ContactsProjectionChecks.kt:42`). Direct inspection of the mapper
output showed deterministic labels `home, work`, while the check expects
`work, home`. This is an acceptance-fixture/assertion mismatch or an ordering
contract decision that needs an owner; it is separate from the callback
tombstone defect.

Calendar callback checks could not execute with the plain JVM harness because
`CalendarSyncCallback` constructs Android `ContentValues`, and the compile SDK
`android.jar` is a stub at runtime:

```text
java.lang.RuntimeException: Stub!
at android.content.ContentValues.<init>(ContentValues.java:23)
```

This is an environment limitation. The Calendar acceptance object is also not
JUnit-discovered. It needs either a Robolectric/Android framework test runtime
or an API-35 emulator instrumentation harness before its assertions can be
claimed as executed.

## Disposition

- **Build/package:** pass.
- **Gradle unit-test task:** pass as a task, but discovered zero tests.
- **Contacts callback:** partial; three non-tombstone checks pass, tombstone
  persistence exposes a product defect.
- **Calendar callback:** not executed; plain-JVM Android stub limitation.
- **Provider/Anytype safety:** pass; no provider writes, Anytype access, or
  credentials were used.

Next action is to fix the Contacts tombstone revision propagation, decide the
canonical email ordering (or correct its fixture), then add an executable
test runtime for the Calendar callback checks. No live provider or Anytype
validation should proceed on the basis of this report alone.

## Re-run after callback changes (2026-09-20)

This re-run used the existing stopped sparse VM
`any-cal-contacts-callback-20260920` rather than the host Flatpak or a
personal Android profile. The VM was started, the current `android/` module
and no-write scripts were copied into `/workspace/android/android`, and the
VM was cleaned up after evidence capture. No emulator, provider account,
provider row, Anytype endpoint, or credential was used.

The VM already had JDK 17 and Android SDK metadata, but the pinned Gradle
binary was absent (`/opt/gradle` was empty). That is why provisioning was
needed; the initial Gradle invocation failed before Gradle started with:
`/opt/gradle/gradle-8.10.2/bin/gradle: not found`. The repository script
`scripts/android-native-build/provision-toolchain.sh` then provisioned the
declared sparse-VM toolchain: Gradle 8.10.2, API 35, Build-Tools 34.0.0,
NDK 27.2.12479018, JDK 17, Rust stable, and cargo-ndk. Provisioning
completed successfully. The sdkmanager XML-version warning remained
non-fatal.

### Build gate

Command, run inside the disposable VM:

```sh
cd /workspace/android/android
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon --console=plain \
  :app:assembleDebug :app:testDebugUnitTest
```

Result: **BUILD SUCCESSFUL**, 39 actionable tasks, in 2m 5s. The APK was
created with SHA-256:

```text
07b8c7aa3f6ea35ae952952007aa7d1c1d219e39b950992f83c0227c045c1e73
```

The Gradle HTML report records **0 tests, 0 failures, 0 ignored**. A green
`testDebugUnitTest` task therefore remains insufficient evidence that the
plain Kotlin acceptance objects ran.

### Direct callback checks

The compiled classes were invoked directly with the API-35 `android.jar` and
Kotlin 2.0.21 standard library on the temporary JVM classpath:

| Check | Result | Evidence |
| --- | --- | --- |
| `ContactsProjectionChecks.main()` | **PASS** | Exit 0; deterministic labels, groups, hash, tombstone, identity, and capability checks passed. |
| `ContactsSyncCallbackChecks.main()` | **FAIL** | Exit 1 at `ContactsSyncCallbackChecks.kt:94`; tombstone binding did not retain revision `"2"`. |
| `CalendarSyncCallbackAcceptanceTest.INSTANCE.runAll()` | **PASS** | Exit 0 through a temporary JShell invocation; all four provider-free validation groups passed. |

The Contacts tombstone defect is still present in the current source. The
planner creates a synthetic deleted `ContactRecord` carrying the tombstone
revision, but the apply path later looks only in `byId`; for a tombstone that
lookup is null, so the saved `ProjectionBinding` receives no
`tombstoneRevision`. This is a product defect and remains a gate for Contacts
callback acceptance. The email-order assertion is now passing in the direct
projection check (`home, work`).

Calendar’s provider-free callback seam is executable and passes its direct
acceptance run. This does not claim live `CalendarContract` writes; those
still require a separate Android framework/instrumentation gate.

### Disposition

- **Toolchain provisioning:** pass after the documented missing-Gradle
  prerequisite was identified.
- **APK assembly:** pass.
- **Gradle unit-test task:** pass as a task, but discovered zero tests.
- **Contacts projection:** pass.
- **Contacts callback:** **blocked by tombstone-revision persistence defect**.
- **Calendar callback validation seam:** pass directly; live provider writes
  remain untested.
- **Safety:** pass; no Anytype or provider writes and no credentials used.

## Re-run after Contacts tombstone lookup fix (2026-09-20)

The current source was copied into a new sparse disposable VM,
`any-cal-callback-rerun-20260920`. No host Flatpak, personal Android
profile, emulator, provider account, provider row, Anytype endpoint, or
credential was used. The VM was removed after the checks below.

The first invocation used `/workspace/android` and exposed the copy layout
(`android/` was nested under that path), so it failed before Gradle started
with “Directory `/workspace/android` does not contain a Gradle build.” The
same invocation was immediately rerun from the actual copied project root,
`/workspace/android/android`; this was an environment-path correction, not a
product failure.

The pinned sparse toolchain provisioning completed successfully using
`scripts/android-native-build/provision-toolchain.sh`: JDK 17, Gradle 8.10.2,
API 35, Build-Tools 34.0.0, NDK 27.2.12479018, Rust stable, and cargo-ndk.
The existing sdkmanager XML-version warning was non-fatal.

### Build and direct checks

From `/workspace/android/android`:

```sh
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon --console=plain \
  :app:assembleDebug :app:testDebugUnitTest
```

Result: **BUILD SUCCESSFUL**, 39 actionable tasks, in 2m 8s. The APK SHA-256
was:

```text
c6e7f79b61f37c867bc1bab2b8d1baa475525a827f0b415a3b297932a0586fe8
```

The Gradle HTML report records **0 tests, 0 failures, 0 ignored**. The plain
Kotlin acceptance objects were therefore invoked directly using the compiled
classes, API-35 `android.jar`, and Kotlin 2.0.21 standard library:

| Check | Result | Evidence |
| --- | --- | --- |
| `ContactsProjectionChecks.main()` | **PASS** | Exit 0. |
| `ContactsSyncCallbackChecks.main()` | **PASS** | Exit 0; tombstone binding revision propagation now passes. |
| `CalendarSyncCallbackAcceptanceTest.INSTANCE.runAll()` | **PASS** | Exit 0 through a temporary JShell invocation. |

This rerun proves the provider-free callback acceptance seam after the
tombstone lookup fix. It does not claim live ContactsContract or
CalendarContract writes; those remain separate Android framework/provider
gates.

### Rerun disposition

- **Build/package:** pass.
- **Direct Contacts projection and callback checks:** pass.
- **Direct Calendar callback checks:** pass.
- **Gradle unit-test task:** task pass, but test discovery remains zero.
- **Provider/Anytype safety:** pass; no writes or credentials used.
