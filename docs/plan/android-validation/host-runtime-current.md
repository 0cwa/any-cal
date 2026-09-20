# Host-accelerated Android runtime validation

Date: 2026-09-20

## Disposition

**The host-accelerated API-35 runtime gate passed.** A fresh Google APIs
x86_64 AVD booted with KVM acceleration, Android framework services became
available, the current APK installed, the account authenticator and both sync
adapters registered, and a disposable app-UID ContentResolver probe exercised
ContactsContract and CalendarContract create/update/tombstone/delete paths.

This run did not use the user's personal Flatpak, Android profile, device, or
Anytype credentials. The probe account and all provider rows were synthetic;
the emulator was stopped and removed after evidence capture.

## Isolated environment

The host had `/dev/kvm` available and passed `msb doctor` virtualization
checks. A temporary SDK/AVD was created under `/tmp/any-cal-android-sdk` and
`/tmp/any-cal-avd`:

```text
Android Emulator 37.1.11.0
API 35 Google APIs x86_64
AVD anycal-api35-host
emulator flags: -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect -accel on -no-snapshot
serial: emulator-5566
sys.boot_completed=1
```

The host SDK used the pinned API-35 platform, NDK 27.2.12479018, Gradle
8.10.2, and a rustup toolchain with the `x86_64-linux-android` target.

## Native runtime evidence

The repository bridge was compiled from `crates/android-bridge` and staged in
the debug APK. The APK installed successfully:

```text
Performing Streamed Install
Success
package:/data/app/.../org.anycal.android-.../base.apk
```

A disposable probe activity loaded the same `libany_cal_android_bridge.so`
and invoked the JNI exports used by `NativeRustBridge`:

```text
I/AnyCalNativeProbe: load=OK schema=1 health=1 json={"schema_version":1,"checkpoint":null,"decisions":[],"error":{"code":"invalid_request","message":"invalid bridge JSON: missing field `account_type` at line 1 column 43"}}
```

This proves `System.loadLibrary`, `nativeSchemaVersion`, `nativeHealth`, and
the credential-free JSON error path on a real Android runtime. The malformed
request was intentionally credential-free.

## Android registration evidence

`dumpsys content` reported both Any-Cal sync adapters:

```text
SyncAdapterType {name=com.android.contacts, type=org.anycal.account, ... packageName=org.anycal.android}
  ComponentInfo{org.anycal.android/org.anycal.android.sync.AnyCalSyncAdapterService}
SyncAdapterType {name=com.android.calendar, type=org.anycal.account, ... packageName=org.anycal.android}
  ComponentInfo{org.anycal.android/org.anycal.android.sync.AnyCalCalendarSyncAdapterService}
```

`dumpsys account` reported the authenticator and the disposable account:

```text
Account {name=anycal-provider-gate-20260920, type=org.anycal.account}
AuthenticatorDescription {type=org.anycal.account, ... packageName=org.anycal.android}
```

The APK requested contacts/calendar permissions. The disposable probe was
granted only those four runtime permissions before provider operations.

## ContentResolver provider gateway evidence

A temporary `ProviderProbeActivity`, installed under the real
`org.anycal.android` package and signed with the disposable debug key, called
the Android `ContentResolver` as the app UID. It used
`CALLER_IS_SYNCADAPTER=true`, the synthetic Any-Cal account, and provider
public contracts. It was not a shell-UID or direct database probe.

The final log line was:

```text
I/AnyCalProviderProbe: account_added=false contacts{initial=1,updated=1,tombstone=1,final=0} calendar{initial=1,updated=1,final_events=0,final_calendars=0}
```

Interpretation:

- Contacts: raw contact insertion and data rows succeeded; source identity
  was updated; a `DELETED=1` tombstone was observed; final deletion left no
  synthetic raw-contact row.
- Calendar: calendar and event insertion succeeded; event update succeeded;
  event and calendar deletion left no synthetic rows.
- `account_added=false` means the account already existed from the first
  probe invocation, not that account creation was unsupported.

Post-run provider queries by the synthetic account returned `No result found`
for both `com.android.contacts/raw_contacts` and
`com.android.calendar/calendars`.

One important Android-specific correction was observed during the probe:
calendar sync-adapter URIs must include `account_name` and `account_type` as
URI query parameters in addition to `caller_is_syncadapter=true`. Omitting
those parameters produced the provider's exact error:

```text
IllegalArgumentException: Sync adapters must specify an account and account type
```

The product calendar gateway must retain this URI requirement.

## Tasks.org capability probe

The clean API-35 image did not contain Tasks.org:

```text
pm path org.tasks -> no output
content query content://org.tasks.api/v0/tasks -> Could not find provider: org.tasks.api
```

No Tasks.org rows or writes were attempted. The optional Tasks.org adapter
therefore remains version/capability-gated and unsupported in this runtime.

## Scope boundary and next gate

This proves the Android account/provider boundary and JNI runtime, not live
Anytype transport or a completed product reconciliation loop. The current
foundation sync callbacks still use the no-op bridge; the disposable probe
directly exercised the same app-UID `ContentResolver` gateway to establish
provider semantics safely.

The next implementation gate is to replace the no-op bridge with the durable
Anytype reconciliation path, then run the same provider assertions through
`onPerformSync` with checkpoint, restart, conflict, observer-loop, and
account-removal tests.

## Repeat preflight after calendar URI fix (2026-09-20)

The parent-requested repeat was intentionally bounded to a five-minute
preflight. Host KVM is still available (`/dev/kvm`, read/write), but no
Android emulator binary, AVD, or cached API-35 system image exists on the
host after the earlier disposable environment was cleaned up:

```text
command -v emulator -> no result
cached API-35 system image search -> no result
/dev/kvm -> present, mode crw-rw-rw-, group kvm
```

The repeat therefore did not download a 1.7-GB image, did not use nested TCG,
and did not claim a second runtime receipt. The completed host-accelerated
receipt above remains the authoritative live evidence. The repository's
current `CalendarContractStore.syncUri` statically includes all three required
parameters (`caller_is_syncadapter=true`, `account_name`, and `account_type`);
the next repeat should use a pre-provisioned API-35 image or CI emulator and
reproduce the same ContentResolver receipt.

## Fresh host-KVM repeat after Calendar URI fix (2026-09-20)

**PASS.** A new API-35 Google APIs x86_64 image was downloaded to a disposable
workspace-local SDK and booted with KVM acceleration. The run rebuilt the
current source (including the Calendar URI fix), installed a temporary probe
APK under the real `org.anycal.android` package, and ran all provider writes
as the app UID. The emulator was held open for the complete probe and then
stopped; no temporary SDK, AVD, image archive, or probe project remains after
cleanup.

Environment receipt:

```text
Android Emulator 37.1.11.0 (build 15917651)
API 35 Google APIs x86_64
AVD anycal-api35-host
KVM: /dev/kvm present; emulator flag -accel on; no TCG fallback requested
emulator flags: -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect -no-snapshot -wipe-data
sys.boot_completed=1
```

The rebuilt disposable probe APK SHA-256 was:

```text
e59d8b77c9e9e1a2aee03cfb71dce9f58a10e9e13dea28ca6b0e8815af89aff3
```

JNI and provider result from the app log:

```text
PASS native{load=true,schema=true,health=true} account_added=true contacts{initial=1,updated=1,tombstone_write=1,tombstone_rows=1,deleted=1,final=0} calendar{writes=7,event_tombstone_write=1,event_tombstone_rows=1,event_deleted=1,calendar_deleted=1,final_events=0,final_calendars=0} tasks_org_provider=false
```

Contacts therefore passed app-UID create, data insertion, source update,
tombstone, final delete, and account-scoped final absence. Calendar passed
calendar and event create, update, event tombstone, event delete, calendar
delete, and account-scoped final absence. The synthetic account was
`anycal-repeat-20260920`; no personal account or data was accessed.

Every Calendar write URI was captured and independently checked before the
ContentResolver call. All seven writes contained
`caller_is_syncadapter=true`, `account_name=anycal-repeat-20260920`, and
`account_type=org.anycal.account`:

```text
calendar-insert  content://com.android.calendar/calendars?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
calendar-update  content://com.android.calendar/calendars/1?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
event-insert     content://com.android.calendar/events?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
event-update     content://com.android.calendar/events/1?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
event-tombstone  content://com.android.calendar/events/1?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
event-delete     content://com.android.calendar/events/1?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
calendar-delete  content://com.android.calendar/calendars/1?caller_is_syncadapter=true&account_name=anycal-repeat-20260920&account_type=org.anycal.account
```

The app-UID capability probe also reported `tasks_org_provider=false`; the
clean image had no Tasks.org provider. No task rows or task writes were
attempted.

This repeat is stronger than the prior static URI assertion: it exercised the
current Calendar URI construction against the live API-35 Calendar provider,
and the provider accepted every account-qualified sync-adapter write.
