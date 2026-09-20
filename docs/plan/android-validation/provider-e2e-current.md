# Current Android provider E2E validation
Date: 2026-09-20

## Scope and safety boundary

This lane was limited to the current `org.anycal.android` debug APK, a
disposable API-35 Google APIs emulator, and synthetic/provider-free checks. It
did not use the user's Flatpak, Android profile, Anytype credentials, a DAV
server, or a personal device. No provider rows, accounts, contacts, events, or
tasks were created or deleted.

The emulator was hosted in the disposable microsandbox
`any-cal-api35-runtime-8g-20260920`. The sandbox and its emulator stopped (and
the sandbox was subsequently absent from `msb list`) during recovery from a
first-boot failure. No cleanup of a personal environment was required.

## APK build and manifest evidence

The current repository Android module was copied into the disposable VM and
built with the pinned toolchain:

```text
ANDROID_HOME=/opt/android-sdk
ANDROID_SDK_ROOT=/opt/android-sdk
/opt/gradle/gradle-8.10.2/bin/gradle --no-daemon --console=plain :app:assembleDebug
```

The build produced `app-debug.apk` inside the disposable VM. The APK badging
reported:

```text
package: name='org.anycal.android' versionCode='1'
versionName='0.1.0-foundation' targetSdkVersion:'35'
```

`aapt dump xmltree` confirmed these exported services and metadata resources:

| Service | Intent | Metadata | Declared data contract |
|---|---|---|---|
| `org.anycal.android.account.AnyCalAuthenticatorService` | `android.accounts.AccountAuthenticator` | `@xml/authenticator` | account type `org.anycal.account` |
| `org.anycal.android.sync.AnyCalSyncAdapterService` | `android.content.SyncAdapter` | `@xml/syncadapter` | `com.android.contacts`, account type `org.anycal.account` |
| `org.anycal.android.sync.AnyCalCalendarSyncAdapterService` | `android.content.SyncAdapter` | `@xml/calendar_syncadapter` | `com.android.calendar`, account type `org.anycal.account` |

The APK requests `READ_CONTACTS`, `WRITE_CONTACTS`, `READ_CALENDAR`, and
`WRITE_CALENDAR`. This is declarative packaging evidence only; it does not
prove that Android registered the services or that any projection writes work.

## Emulator result

The API-35 Google APIs image and AVD were present in the VM. ADB reached the
emulator transport, but Android's first boot never completed. In particular:

```text
adb get-state                         -> device (during partial boot)
getprop sys.boot_completed             -> empty
getprop sys.bootstat.first_boot_completed -> 0
service list                           -> no package, account, or content service
adb install app-debug.apk              -> failed
  cmd: Can't find service: package
```

Before that failure, the package-install attempt on the reused AVD also
returned Android's package-installer null-service error:

```text
java.lang.NullPointerException: ... PackageManagerInternal.freeStorage(...)
```

The AVD was then restarted with `-wipe-data`; it still remained in first boot
with `system_server`/package services unavailable. The VM terminated while
the clean boot was being recovered. Consequently there is no authoritative
`pm path`, `dumpsys package org.anycal.android`, account-manager registration,
or provider-row result for the current APK.

## Provider CRUD disposition

No deterministic provider create/update/delete/tombstone path was exercised.
That result is intentional rather than a failed CRUD assertion: without a
successful APK install and Android package/account services, a provider-row
observation would not be attributable to Any-Cal.

Static source inspection also shows that the currently built foundation does
not yet perform live reconciliation:

- `AnyCalSyncAdapterService.onPerformSync` probes capabilities and calls the
  no-op Rust bridge; it does not use the supplied `ContentProviderClient`.
- `AnyCalCalendarSyncAdapterService.onPerformSync` likewise calls the no-op
  bridge and performs no calendar writes.
- `ContactsContractAdapter` currently supplies capability checks and the
  `CALLER_IS_SYNCADAPTER` URI boundary, not insert/update/delete operations.
- `CalendarContractStore` contains provider write helpers, but no live sync
  callback currently invokes them.

Therefore the current APK cannot yet support a live one-way Contacts or
Calendar projection claim even if emulator installation succeeds. The next
valid gate must first install the APK, register a disposable
`org.anycal.account` account, grant runtime permissions, and then prove rows
with Any-Cal source IDs through create/update/delete/recreate checks.

## Tasks.org capability probe

No Tasks.org package or provider was installed by this lane. The exact
read-only probe could not run after first boot because Android's package and
content services never became available; no `org.tasks` package/version or
`org.tasks.api` authority claim is made from this run.

The previously recorded isolated API-35 capability probe remains the applicable
separate evidence for the clean image: it found `org.tasks` and
`org.tasks.api` absent. That evidence is recorded in
[`android-toolchain-validation.md`](../android-toolchain-validation.md), not
silently combined with this incomplete current-APK install run.

## Required follow-up gate

Repeat in a fresh disposable API-35 VM with a fully booted system before
claiming provider support:

1. Verify `sys.boot_completed=1`, `service list` contains package/account and
   content services, and `adb install` succeeds.
2. Run `pm path` and `dumpsys package` to prove the two sync adapters are
   registered for `com.android.contacts` and `com.android.calendar`.
3. Add only an `org.anycal.account` synthetic account and grant contacts and
   calendar permissions.
4. Invoke the actual sync path and capture only synthetic row IDs,
   `SOURCE_ID`/`_SYNC_ID`, and operation counts.
5. Prove update, delete/tombstone, restart/reconcile, and account-removal
   behavior; query afterward to prove no synthetic rows remain.
6. Separately run `resolve-content-provider org.tasks.api`, `pm path org.tasks`,
   version, and permission checks. Do not enable the optional adapter unless
   the installed Tasks.org contract passes its version-gated CRUD probe.
7. Stop and remove the disposable VM after evidence capture.
