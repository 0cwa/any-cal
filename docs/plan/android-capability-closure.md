# Android provider, DAV fallback, and Tasks.org capability closure

**Date:** 2026-09-20
**Scope:** documentation reconciliation only. No device, APK, account,
provider, Anytype, or network state was changed for this closure.

## Claims supported by current evidence

| Capability | Supported claim | Evidence boundary |
| --- | --- | --- |
| Direct Contacts projection | The current app-UID probe can create, update, tombstone, delete, and account-scope-clean synthetic ContactsContract rows using the Any-Cal sync account. | This proves provider gateway semantics and account ownership. It does not prove the production `onPerformSync` path or a live Anytype round trip. |
| Direct Calendar projection | The current app-UID probe can create, update, tombstone, and delete synthetic CalendarContract calendars/events. All seven tested writes included `caller_is_syncadapter=true`, `account_name`, and `account_type`. | This proves the tested API-35 provider behavior. It does not prove recurrence, reminders, timezone fidelity, OEM behavior, or production reconciliation. |
| Direct provider ownership | Any-Cal direct mode may own explicitly configured Contacts/Calendar datasets through its own account. | One writer must own a dataset. Direct mode and DAV relay must not both write the same provider rows. |
| DAV fallback | The desktop DAV service has deterministic, sanitized Android-shaped discovery/VTODO/CardDAV replay coverage, including ETags, preconditions, reconnect, and errors. | Replay is in-process and client-shaped; it is not live DAVx5, EteSync, Tasks.org, or Android interoperability evidence. |
| Tasks.org | Tasks.org is optional and runtime/version gated. The clean API-35 image used for the provider probe had neither the `org.tasks` package nor the `org.tasks.api` authority. | No Tasks.org rows or writes were attempted. This is runtime-specific absence, not a platform-wide claim. |

## Ownership modes

Ownership is selected separately for contacts, calendars, and tasks:

1. **Direct:** Any-Cal owns its Android account and provider projection.
2. **DAV relay:** DAVx5/EteSync owns the Android provider rows; Any-Cal must
   not also write those rows.
3. **Import-only:** Any-Cal copies an external account into its own namespace,
   retaining source identity and never silently writing back.

The same logical records may appear in two modes, but that does not make the
provider rows interchangeable. Source IDs, account scope, tombstones,
ownership mode, and hashes remain distinct. A duplicate or missing source ID
must produce a reviewable ambiguity rather than automatic merge or deletion.

## Future live gates

### Tasks.org

Before enabling the optional adapter, use a fresh API-35-or-newer emulator and
a pinned Tasks.org APK (initial target: 15.12 or a later explicitly tested
version). Record only package/version, authority resolution, permission
result, column/schema names, row counts/IDs, return codes, and notification
behavior. The bounded sequence is:

1. Resolve `org.tasks.api` and verify the documented
   `content://org.tasks.api/v0` contract.
2. Check `READ_TASKS` and `WRITE_TASKS` without assuming permission implies
   provider compatibility.
3. Read lists/tasks with bounded projections and paging.
4. Register a collection observer and verify a provider-originated change.
5. Only after the read-only gate passes, obtain separate authorization for
   one marked task/list CRUD sequence: create, update, complete/uncomplete,
   delete, and restart/re-read.

If the authority, package, version, columns, or semantics differ, keep the
adapter disabled and retain the DAV path. Numeric Tasks.org IDs are local
provider IDs, not Anytype identity.

### DAVx5/EteSync

A future live run needs an explicitly disposable emulator/device, pinned
client APKs, a disposable DAV account, and authority to create/delete only
marked records. It must capture the real discovery path before CRUD and test
one client at a time. The run must separately cover discovery, contact/task
CRUD, ETags, reconnect, ownership, and cleanup. Passing the sanitized replay
does not waive these prerequisites.

## Residual gates

- Production Android callbacks still need the durable Anytype bridge and
  checkpoint/restart/conflict evidence.
- Direct Contacts/Calendar recurrence, groups, photos, reminders, timezone,
  OEM scheduling, and provider recreation remain unproven.
- Live DAVx5, EteSync, and Tasks.org compatibility remains unclaimed.
- No Android platform-wide TasksContract is assumed.

Authoritative supporting artifacts:

- [`android-fallback-interoperability.md`](android-fallback-interoperability.md)
- [`android-account-lifecycle.md`](android-account-lifecycle.md)
- [`android-validation/host-runtime-current.md`](android-validation/host-runtime-current.md)
- [`tasks-org-provider-probe.md`](tasks-org-provider-probe.md)
- [`work-units/android-dav-client-profile.json`](work-units/android-dav-client-profile.json)
