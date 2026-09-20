# Tasks.org provider probe

**Date:** 2026-09-19  
**Disposition:** blocked on disposable Android emulator availability

## Scope

Determine whether a current Tasks.org build exposes the documented provider
`content://org.tasks.api/v0`, and validate provider discovery, permissions,
schema, change notifications, and bounded CRUD. This probe must use only a
disposable API-35-or-newer emulator and disposable task records. It must not
contact Anytype or use a personal Android profile.

## Environment evidence

The requested probe could not run in this lane:

```text
$ adb devices -l
List of devices attached

$ emulator -list-avds
/bin/bash: emulator: command not found
```

The first `adb` attempt could not start the daemon in the restricted shell;
the approved host-context retry started successfully but reported no devices.
No emulator binary or AVD profile is available on the host. Therefore there
is no installed Tasks.org version, package metadata, provider resolution,
permission result, or CRUD result to report.

## Deterministic probe design

When a disposable API-35+ emulator and a current Tasks.org APK (15.12 or
newer) are available, record:

1. `adb shell getprop ro.build.version.sdk` and `adb shell getprop ro.build.id`.
2. `adb shell pm path org.tasks` and `adb shell dumpsys package org.tasks`.
3. Resolve `org.tasks.api` with `PackageManager.resolveContentProvider` or
   `adb shell cmd package resolve-content-provider org.tasks.api`.
4. Check declared/granted `org.tasks.permission.READ_TASKS` and
   `org.tasks.permission.WRITE_TASKS`; request them only after explicit
   disposable-account approval.
5. Read `content://org.tasks.api/v0/lists` and `/v0/tasks` with bounded
   projection and paging. Verify the documented empty-value conventions and
   `EXTRA_TOTAL_COUNT`.
6. Register a `ContentObserver` on `content://org.tasks.api/v0` and verify a
   provider-originated change notification.
7. Create exactly one uniquely marked task in a disposable list, then read,
   patch, complete/uncomplete, and delete it. Record only row IDs, counts,
   return codes, and schema/column names; never record task content or
   credentials.
8. Repeat the read-only and permission checks after Tasks.org process restart.

The probe must fail closed if the authority is absent, permissions are
denied, a URI/column differs, or the installed version is outside the tested
support matrix. It must not fall back to the legacy `org.tasks` provider,
whose current source is query-only and not a CRUD compatibility API.

## Current contract evidence

Tasks.org publishes `CONTENT_PROVIDER.md` in its source repository. It states:

- authority `org.tasks.api` and base URI `content://org.tasks.api/v0`;
- permissions `org.tasks.permission.READ_TASKS` and `WRITE_TASKS`;
- collection CRUD for tasks, reminders, lists, tags, places, and task-tags;
- paging, projections, named query parameters, and `EXTRA_TOTAL_COUNT`;
- collection-level `ContentObserver` notifications;
- local numeric IDs and version `v0` explicitly marked unstable.

Reference: <https://raw.githubusercontent.com/tasks/tasks/main/CONTENT_PROVIDER.md>

The provider is therefore a plausible optional adapter, but it is not an
Android platform contract. Any-Cal must resolve the authority and inspect the
installed package/version at runtime. Contacts and Calendar synchronization
must remain independent if this adapter is unavailable.

## Unresolved compatibility risks

- No live evidence yet that the current APK publishes the documented
  authority.
- The API is explicitly unstable; columns and semantics may change between
  releases.
- Numeric provider IDs are local to one Tasks.org installation and cannot be
  used as Anytype identity.
- Completing recurring tasks mutates the series and advances dates rather than
  simply setting a completion timestamp.
- Deleting a task recursively deletes subtasks and cannot be undone.
- Read-only lists/accounts may reject writes.
- Change notifications carry no payload and require a fresh bounded query.

## Integration disposition

Do not claim Tasks.org direct integration until the disposable probe above
passes against a pinned APK version. Until then, use the existing CalDAV path
for Tasks.org and implement native Android providers only for Contacts and
Calendar.
