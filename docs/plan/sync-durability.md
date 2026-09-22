# Sync durability

The `any-cal-sync` crate is the conservative persistence boundary for sync
bookkeeping. It stores a versioned checkpoint containing observed resource
identity (`Anytype object ID`, DAV UID, revision, and ETag), pending idempotent
operations, and tombstones. Checkpoints are written to a uniquely named
temporary file, fsynced, renamed into place, and backed up with backup and
parent-directory fsync. A process-level create-new lock enforces one writer
per checkpoint. State is staged in a candidate clone and only published after
the checkpoint succeeds. If the primary checkpoint is corrupt, the previous
backup is loaded; indexes remain rebuildable maps rather than authoritative
state.

Pending operations are not automatically retried after conflicts, malformed
state, or other non-transient failures. A caller may increment an attempt only
for a classified transient failure. Archive/delete operations remove the
observed entry and leave a tombstone, preventing accidental resurrection until
an explicit new create decision is made. Filesystem write, rename, sync, and
crash-like pre-rename failures are injectable in tests, and symlink checkpoint
paths are rejected.

This layer does not claim an Anytype change stream, sync token, or transaction
semantics. Until live validation is authorized, change detection must use
polling plus omission-safe reconciliation, and the adapter must supply the
transient/error classification. Platform-specific file-lock durability and
live server conflict behavior remain follow-up research after the Anytype
probe.

The app integration is opt-in through `AppConfig.sync_checkpoint` (or
`ANY_CAL_SYNC_CHECKPOINT`/`--sync-checkpoint`). Successful DAV PUT/DELETE
operations refresh the visible Contact/VTODO set into this checkpoint; rows
that disappear after archive/delete become tombstones. A checkpoint failure
returns a bounded service error and does not publish the candidate state in
memory. The service still treats the Anytype repository as authoritative and
does not claim live change streams or distributed transactions.

Normal snapshot refresh rejects a resource ID that is already tombstoned with
a typed resurrection error. Reuse requires the explicit
`replace_observed_explicit_resurrection` API, which atomically removes the
tombstone and publishes the new observed record as a deliberate create.


## Binding-scoped checkpoints

For multi-domain routing, `SyncStore::open_scoped` binds an entire checkpoint to a
non-secret `SyncScope` containing:

- domain ID;
- domain-binding fingerprint;
- hashed endpoint identity;
- upstream account fingerprint;
- Space ID.

The checkpoint therefore owns its observed set, pending operations, and tombstones as
one binding-qualified unit. Reopening it with another Space, account, endpoint, or
binding fingerprint fails closed. The legacy unscoped `SyncStore::open` API refuses
to open an already-scoped checkpoint, so older call sites cannot silently bypass the
scope.

An existing **non-empty** unscoped checkpoint cannot silently adopt a scope because its
prior resources may belong to another binding. Empty state may adopt a scope. Export
payloads carry the scope inside the integrity-protected state, and in-process restore
requires an exact scope match before publication.

This API is the persistence seam for the Space-qualified routing work in issue #3.
The current single-domain application now derives the exact `legacy-default` binding
plus a non-secret upstream account-context fingerprint at startup and opens configured
checkpoints with `open_scoped`. Reusing a checkpoint after changing Space, endpoint,
or credential context therefore fails closed.

The same resolved binding context is also passed into `AnytypeRepository`. Repository
lock keys and operation IDs are qualified by the binding fingerprint, and adapter reads,
writes, archive/delete confirmation, and ambiguous-write reconciliation reject transport
responses whose Space or requested object identity does not match the bound repository.
Explicit multi-domain configuration remains disabled until DAV route selection can choose
one of these binding-scoped repository contexts without falling back to scalar `space_id`.
