# Android release lifecycle validation

Date: 2026-09-20

## Scope and isolation

## Reconciliation note (2026-09-20)

The x86_64-only APK described below is the earlier lifecycle receipt and is
retained as historical evidence. The later
[`bridge-integration-current.md`](bridge-integration-current.md) receipt
asserts both `arm64-v8a` and `x86_64` entries in the current release APK and
passes the corresponding build/verifier checks. This does not close real
AccountManager add/remove/re-add behavior or backup/restore: `allowBackup=false`
remains explicit and those release gates remain pending.

This lane used a fresh host-KVM API-35 Google APIs x86_64 emulator. The
temporary SDK, AVD, build outputs, and provider rows were all under
`/tmp/any-cal-release-20260920` and were removed after evidence capture. The
run did not use the user's Flatpak, Android profile, personal device, Anytype
credentials, live Tasks.org, or a non-synthetic provider account.

Environment receipt:

```text
Android Emulator 37.1.11.0 (build 15917651)
API 35 Google APIs x86_64
KVM: /dev/kvm available; emulator QEMU command included -enable-kvm
AVD: anycal-release-api35
emulator: -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect
           -accel on -no-snapshot -wipe-data
sys.boot_completed=1
```

The first background launch exited before ADB became available. A foreground
retry reached `sys.boot_completed=1`; no result from the failed first launch
was counted as lifecycle evidence.

## Build and package gate

The current repository APK was rebuilt using JDK 17, Gradle 8.10.2, Android
platform 35, and Build-Tools 34.0.0:

```text
BUILD SUCCESSFUL in 24s
41 actionable tasks: 41 executed
```

Current APK:

```text
SHA-256  688cecc684e6c42902158cf8809ffddde1c46ec6076088ca2633b34cb56f6f64
package  org.anycal.android
version  1 / 0.1.0-foundation
min SDK  26
target   35
```

`apksigner verify --verbose` passed with APK Signature Scheme v2. The APK
contains `lib/x86_64/libany_cal_android_bridge.so`; it does not contain an
`arm64-v8a` library even though the Gradle ABI filter declares both ABIs. This
is a release-packaging gap: arm64 runtime support must not be claimed until a
current build includes and loads the arm64 library.

The merged manifest declares the Contacts and Calendar read/write permissions,
the authenticator, and both sync-adapter services. `allowBackup=false` is
present in the merged manifest. Consequently Android backup/restore is
explicitly disabled for this build; no backup/restore receipt is claimed.

## Install, upgrade, restart, and reinstall

A disposable version-1 baseline APK (`0.0.9-lifecycle-baseline`, version code
1) was built from the same source in a temporary copy. A temporary version-2
APK (`0.1.0-foundation`, version code 2) was then built from the current
source copy solely to exercise package-manager upgrade behavior. Neither
temporary version was added to the repository.

Results:

| Lifecycle operation | Result | Evidence |
|---|---|---|
| Clean install baseline | PASS | `adb install` returned `Success`; package reported version code 1. |
| Upgrade baseline → version 2 | PASS | `adb install -r` returned `Success`; package reported version code 2. |
| Force-stop/restart persistence | PASS | Synthetic contact/calendar rows remained queryable after `am force-stop`. |
| Uninstall | PASS for non-destructive behavior | `adb uninstall` returned `Success`; provider rows were not removed. |
| Reinstall current APK | PASS | `adb install` returned `Success`; package reported version code 1. |
| State after reinstall | PASS | Synthetic Any-Cal and foreign rows remained unchanged. |

The post-reinstall Android registration receipt included:

```text
com.android.contacts / org.anycal.account → AnyCalSyncAdapterService
com.android.calendar  / org.anycal.account → AnyCalCalendarSyncAdapterService
org.anycal.account → AnyCalAuthenticatorService
```

## Provider preservation and unrelated-row safety

Only synthetic rows were used. Before the upgrade, the probe created:

- Contacts raw contact with `account_name=synthetic-release`,
  `account_type=org.anycal.account`, `sourceid=release-contact-upgrade`.
- Calendar with the same synthetic account and `_sync_id=
  release-calendar-upgrade`.
- An unrelated Contacts raw contact with
  `account_name=foreign-account`, `account_type=foreign.type`, and
  `sourceid=foreign-contact-preserve`.

All three active rows were present after upgrade, force-stop, uninstall, and
reinstall. No Any-Cal component ran a provider deletion during this lane, and
the unrelated foreign row was never removed. The direct provider rows were
created with the Android shell's `content` utility; this proves package
lifecycle preservation, not that the current no-op sync callback performs a
complete Anytype projection.

## Gaps and disposition

- The current authenticator deliberately returns “account setup is not
  implemented”; therefore this lane could not add a real AccountManager
  account or invoke `onPerformSync` with an account. Account creation,
  account-removal cleanup, and live Anytype reconciliation remain unverified.
- Backup/restore is intentionally unavailable because `allowBackup=false`.
  If backup is required later, it needs an explicit data-extraction policy and
  a separate restore test.
- Only x86_64 native code is packaged in the current APK. Arm64 packaging and
  runtime loading remain a release blocker.
- Tasks.org was not installed or accessed. Its optional provider remains
  capability/version gated.
- No Anytype endpoint, token, personal data, DAVx5, or user profile was used.

The emulator process was stopped and the disposable workspace removed after
these checks.
