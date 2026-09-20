# Local DAV/sync export and restore contract

Status: complete for the bounded provider-free lane.

This lane adds an explicit operator-triggered export/restore format for local
sync bookkeeping. The artifact contains only the versioned `SyncState`
payload; it does not contain credentials, transport configuration, Anytype
secrets, or audit-log contents. Automatic retention/rotation and external
interoperability remain out of scope.

## Contract implemented

- `SyncStore::export_to` writes an `any-cal.sync-export` version-1 envelope.
- The envelope records state/export versions and a SHA-256 digest over the
  canonical serialized payload.
- `SyncStore::restore_from` validates format, versions, state invariants, and
  the digest before publishing the raw checkpoint payload.
- Publication uses a private temporary file, `fsync`, rename, private mode
  `0600`, and parent-directory sync. Validation or staging failure leaves an
  existing destination unchanged.
- Symlink components, malformed/truncated JSON, incompatible versions, and
  digest mismatches fail closed.
- Observed ETags, DAV modification timestamps, revisions, tombstones, and
  pending operations are serialized and recovered. Modification timestamps
  are represented as Unix seconds in sync state and sourced from the core
  repository's `ModifiedAt` value.

## Evidence

Executed with `LD_PRELOAD` unset and a fresh temporary Cargo target:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-export-restore-target-20260920 \
  cargo fmt --check
  passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-export-restore-target-20260920 \
  cargo test -p any-cal-sync --no-fail-fast
  21 passed; 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-export-restore-target-20260920 \
  cargo clippy -p any-cal-sync --all-targets -- -D warnings
  passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-export-restore-target-20260920 \
  cargo test -p any-cal-app --no-run
  passed
```

The focused tests cover valid round-trip/restart, ETag and modification-time
preservation, tombstones, pending state, tamper detection, truncation,
unsupported version, symlink refusal, atomic non-mutation, and private modes.

The ordinary app listener tests were not used as evidence for this local lane;
the restricted sandbox denies their socket operations. No network, external
client, Android provider, DAVx5/Tasks.org, Anytype service, credential, or
personal Flatpak state was accessed.

## Reproduced and fixed defect

The first implementation wrote the validated export envelope to the restore
destination. Reopening that destination treated it as a malformed checkpoint
and recovered an empty state. The restore path now writes only the validated
`SyncState` payload; the round-trip test proves restart recovery.

