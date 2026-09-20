# Local DAV/sync service export and restore integration

Status: complete for the bounded provider-free service lane (2026-09-20).

The local service now exposes an authenticated operator boundary for the
versioned `any-cal.sync-export` contract. The boundary is deliberately narrow:
it uses the configured checkpoint's adjacent `.export` path, does not accept
operator-supplied filesystem paths, and does not implement automatic
retention, rotation, scheduling, or deletion.

## Service boundary

- `GET /admin/sync/capabilities` (also `GET /admin/sync`) reports whether the
  configured checkpoint enables export and restore, plus the fixed format and
  export version.
- `POST /admin/sync/export` atomically publishes the current sync state to the
  adjacent export artifact.
- `POST /admin/sync/restore` validates and atomically restores that artifact
  into the active checkpoint and in-memory `SyncStore`.
- All three operations require the configured global operator credential;
  local health-only credentials do not authorize them. Requests with a body,
  unsupported methods, missing checkpoint, or invalid credentials receive
  bounded errors.
- Responses contain only operation, format/version, generation, and bounded
  item counts. They never include checkpoint paths, resource IDs, collection
  names, credentials, or artifact contents.
- `/health`, `/status`, and `/ready` retain their existing shape when sync is
  not configured; when configured, they add `sync_export_ready: true`.

`SyncStore::restore_export` was added so a running service cannot restore the
file while continuing with stale in-memory state. The validated payload is
published first; only then is the active state replaced. Validation,
integrity, symlink, malformed-body, or atomic-write failures leave the active
state unchanged.

## Evidence

Commands ran with `LD_PRELOAD` unset, offline, and a disposable Cargo target:

```text
env -u LD_PRELOAD cargo fmt --all -- --check
passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo test -p any-cal-app --test app --offline
42 passed; 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo clippy -p any-cal-app --tests --offline -- -D warnings
passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo test -p any-cal-sync --offline
21 passed; 0 failed
```

The focused integration test proves: unauthenticated denial, capability
discovery, authenticated export, authenticated restore, restart-safe in-memory
state replacement, tampered-artifact rejection without mutation, malformed
request rejection, symlink rejection, bounded responses, and diagnostic
redaction. The existing sync contract suite separately covers incompatible,
truncated, digest-invalid, and atomic restore cases.

No network, Anytype state or credential, Android/provider state, DAVx5,
Tasks.org, personal Flatpak state, pending credential artifact, or retention
policy was accessed.

## Deferred boundaries

This receipt does not claim scheduled backups, retention/rotation, encryption
at rest, multi-user administrative policy, external-client interoperability,
live Anytype interoperability, or Android validation.
