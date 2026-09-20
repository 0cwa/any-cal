# Android bidirectional reconciliation build validation

Date: 2026-09-20

## Scope and isolation

This validation used a fresh sparse microsandbox, `any-cal-android-bidir-20260920`:

- Ubuntu 24.04
- 4 vCPUs (maximum 4)
- 4 GiB initial / 8 GiB maximum memory
- 32 GiB managed sparse root disk
- pinned JDK 17, Gradle 8.10.2, Android SDK platform 35, Build-Tools 34.0.0,
  NDK 27.2.12479018, Rust stable, and cargo-ndk 4.1.2

Only the `android/` module and the no-write Android build/probe scripts were
copied into the VM. No personal Flatpak, Android profile, emulator, provider
account, Anytype endpoint, credential, or provider row was mounted or used.

The VM was stopped and removed after evidence capture.

## Gradle build and test task

The requested harness was run from the copied repository layout:

```sh
export PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/root/.cargo/bin:$PATH
export ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk
export GRADLE_BIN=/opt/gradle/gradle-8.10.2/bin/gradle
cd /workspace
scripts/android-build/build-and-test.sh
```

Result: **PASS**.

```text
BUILD SUCCESSFUL in 2m 6s
41 actionable tasks: 41 executed
Tasks.org contract checks passed: no provider access or CRUD performed
APK_SHA256=6bd8460e227ec5d25d9ecad066e3f3ed37c22a385970944bd14af734b9f982af
package: name='org.anycal.android' versionCode='1' versionName='0.1.0-foundation'
platformBuildVersionCode='35' compileSdkVersion='35' compileSdkVersionCodename='15'
android-build assemble/unit/verifier: PASS
```

The build emitted two non-fatal packaging diagnostics:

```text
Unable to strip the following libraries, packaging them as they are: libany_cal_android_bridge.so.
There are no .so files available to package in the APK for arm64-v8a.
```

The APK still assembled successfully. The debug APK was 3.6 MiB with the
checksum above.

Gradle's `:app:testDebugUnitTest` task passed, but the report directory
contained only binary results and **zero XML/discovered tests**. The acceptance
objects below are plain Kotlin entrypoints, so the green Gradle task is not
evidence that those assertions ran.

## Direct provider-free acceptance entrypoints

The compiled classes were invoked directly with the API-35 `android.jar`,
Kotlin 2.0.21 standard library, and the debug/debugUnitTest class directories.
No ContentResolver, Android provider, or Anytype endpoint was used.

| Entry point | Result | Evidence |
| --- | --- | --- |
| `ContactsProjectionChecks.main()` | **PASS** | Exit 0. |
| `ContactsSyncCallbackChecks.main()` | **PASS** | Exit 0; callback persistence/tombstone checks passed. |
| `CalendarBidirectionalReconcilerAcceptanceTest.INSTANCE.runAll()` | **PASS** | Exit 0 through a temporary Java runner. |
| `CalendarSyncCallbackAcceptanceTest.INSTANCE.runAll()` | **PASS** | Exit 0 through a temporary Java runner. |
| `ContactsReconciliationChecks.main()` | **FAIL** | Exit 1 at `ContactsReconciliationChecks.kt:80`. |

The failing Contacts check is deterministic and reproducible. Its test creates
a checkpoint with `lastSourceId = "b"`, while the provider source IDs are
`android/<account-type>/<account-name>/b` and `/c`. The implementation compares
the full source IDs to the bare checkpoint value, so the first source is not
filtered out and the returned checkpoint remains the `b` source instead of
advancing to `c`. The contract must be made consistent: either checkpoints
must store/compare the full opaque source ID, or filtering must extract a
canonical ordering key before comparison. This is an unresolved Contacts
bidirectional integration defect; no source was edited during this validation.

The Calendar bidirectional seam passed echo suppression, local edit and
tombstone emission, generation/batch rejection, and deterministic checkpoint
recovery. The Calendar callback seam also passed.

## No-write probe

The deterministic provider capability probe passed:

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

## Disposition and remaining gaps

- Android compilation and debug APK assembly: **pass**.
- Gradle unit-test task: **pass as a task; zero tests discovered**.
- Contacts projection and callback checks: **pass**.
- Calendar bidirectional reconciler and callback checks: **pass**.
- Contacts provider reconciliation: **blocked by source-ID/checkpoint namespace
  mismatch**, exposed by `ContactsReconciliationChecks`.
- Live ContactsContract/CalendarContract provider writes: **not tested** in this
  provider-free VM and must not be inferred from these results.
- Anytype access and credentials: **not used**.

The next implementation gate is to choose and enforce one checkpoint identity
contract, add a direct regression check for full-source checkpoint progression,
then rerun this same sparse build and acceptance lane before live provider
validation.

## Post-fix rerun

Date: 2026-09-20

The full-source checkpoint fix was rerun in a new sparse VM,
`any-cal-android-bidir-rerun-20260920`, using the same Ubuntu 24.04, 4-CPU,
4/8-GiB, 32-GiB sparse recipe. The VM was removed after evidence capture.

Provisioning installed the pinned JDK, Gradle, SDK, NDK, Rust, and cargo-ndk
toolchain. The provisioning script returned exit 1 only at its final
informational `cargo-ndk --version` command: cargo-ndk intentionally refuses
to run as a standalone binary and reports “This binary may only be called via
cargo ndk”. The binary was installed and the Gradle build used it
successfully; this is a provisioning-script probe defect, not a build failure.

The post-fix build command completed successfully:

```text
BUILD SUCCESSFUL in 2m 4s
41 actionable tasks: 41 executed
Tasks.org contract checks passed: no provider access or CRUD performed
APK_SHA256=979af9e3d7fdc6c23c946685fcc3535af992904f71cd14ecb72e5c0592f9c3ba
package: name='org.anycal.android' versionCode='1' versionName='0.1.0-foundation'
platformBuildVersionCode='35' compileSdkVersion='35' compileSdkVersionCodename='15'
android-build assemble/unit/verifier: PASS
```

The Kotlin compiler daemon emitted a transient startup message and Gradle
continued successfully. The existing non-fatal native packaging diagnostics
remain unchanged. The test report again contained binary results only and
zero XML/discovered tests.

Direct provider-free entrypoints all passed:

```text
ContactsProjectionChecks.main()                         EXIT=0
ContactsSyncCallbackChecks.main()                       EXIT=0
ContactsReconciliationChecks.main()                     EXIT=0
CalendarBidirectionalReconcilerAcceptanceTest.runAll()  EXIT=0
CalendarSyncCallbackAcceptanceTest.runAll()             EXIT=0
```

The no-write probe also passed:

```text
android-probe fake-device validation: PASS
android-probe offline validation: PASS
```

The previous Contacts checkpoint defect is therefore resolved at the
provider-free reconciliation seam. Live ContactsContract/CalendarContract
provider writes and Anytype transport remain separate unverified gates.
