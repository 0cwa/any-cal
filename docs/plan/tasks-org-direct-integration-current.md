# Tasks.org direct provider integration

**Date:** 2026-09-20
**Status:** implemented as a version-gated optional projection; live APK
validation remains required before enabling it by default.

Tasks.org's current source documents a provider introduced in 15.12
(versionCode 151202):

- authority `org.tasks.api`;
- base URI `content://org.tasks.api/v0`;
- dangerous permissions `org.tasks.permission.READ_TASKS` and
  `org.tasks.permission.WRITE_TASKS`;
- task/list/tag/reminder CRUD and collection-level `ContentObserver` signals.

The v0 contract is explicitly unstable. Any-Cal therefore allow-lists only
151202 until a later version is revalidated. The Android manifest declares the
permissions and provider visibility query. `TasksOrgAdapter` performs the
authority, package, version, and permission gate; `ContentResolverTasksOrgGateway`
performs the background-thread schema probe and bounded CRUD; and
`TasksOrgSyncRunner` applies only bridge-authorized task resources.

Canonical Anytype/DAV IDs remain separate from Tasks.org numeric row IDs. A
durable projection binding stores the numeric row ID and observed projection
hash. Update and delete operations fail closed when the row no longer matches
the stored hash, preventing a reused provider ID from modifying or deleting an
unrelated task. Tombstones only act on an existing binding and never scan or
delete unrelated Tasks.org rows.

The current mapping covers title, notes, due/start timestamps and all-day
flags, completion timestamp, recurrence text, list ID, and parent ID. Unknown
VTODO properties stay in the bridge envelope. Recurrence completion behavior,
reminders, tags, list creation, and provider-to-Anytype edits remain separate
validation gates because the provider documents provider-specific side effects
and a retry of recurring completion can advance a series twice.

The required live gate is a disposable API-35+ emulator with a pinned Tasks.org
15.12 APK: resolve the authority, verify permissions and schema, read bounded
lists/tasks, register a collection observer, then run one marked CRUD sequence
and restart/re-read. Until that gate passes, the adapter remains optional and
CalDAV remains the generic Android task path.

Primary contract: <https://raw.githubusercontent.com/tasks/tasks/main/CONTENT_PROVIDER.md>.
