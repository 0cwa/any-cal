# Local sync-admin audit backup and restore

Status: complete for the provider-free, disposable synthetic scope of
`dav-local-sync-admin-audit-backup-restore`.

Date: 2026-09-20

## Evidence

- `env -u LD_PRELOAD cargo test -p any-cal-sync --lib -- --nocapture`: 21
  passed.
- `env -u LD_PRELOAD cargo test -p any-cal-observability --lib -- --nocapture`:
  26 passed.
- `env -u LD_PRELOAD cargo test -p any-cal-app --test app sync_admin --
  --nocapture`: 4 passed.
- `env -u LD_PRELOAD cargo test -p any-cal-app --test app recovery --
  --nocapture`: 4 passed.
- `env -u LD_PRELOAD cargo fmt --all -- --check`: passed.
- `env -u LD_PRELOAD cargo clippy -p any-cal-sync --all-targets -- -D
  warnings`: passed.
- `env -u LD_PRELOAD cargo clippy -p any-cal-observability --all-targets --
  -D warnings`: passed.

## Covered behavior

The sync-store tests use synthetic observed resources, pending operations,
tombstones, sequence/generation state, and correlation-safe receipts. They
prove that an explicit export contains a versioned format and state envelope,
canonical payload digest, bounded counts, and no credentials or transport
configuration. Restore validates the envelope and digest before atomic
publication, preserves order and state, and sets private `0600` modes.

The failure matrix rejects truncated or malformed JSON, unsupported export
versions, state-version mismatches, payload tampering, unsafe/symlink paths,
and incomplete state without changing the existing destination. The active
store path also preserves a validated backup across failed republish attempts,
recovers from a corrupt primary, rejects an unrecoverable primary/backup pair,
and removes stale locks on failed open. Complete malformed audit tails are
rejected while only genuinely incomplete EOF tails are recovered.

The app tests cover authenticated operator capability/export/restore behavior,
request-body rejection, failed restore with unchanged active state, audit
event persistence, redaction, correlation, bounded readback, required/optional
health behavior, restart, and stale-lock recovery. Recovery wrappers emit
bounded backup/restore events. No raw audit message, resource path, credential,
or checkpoint path is returned in receipts.

No local code defect was reproduced in this bounded lane; therefore no source
change was necessary.

## Scope boundaries

All state was synthetic and disposable. This lane did not access live
Anytype, external clients or network, Android/provider state, DAVx5/Tasks.org,
the personal Flatpak, real credentials, the pending credential-bearing
artifact, scheduled backups, automatic retention/rotation/deletion, or
production key management. Temporary test fixtures were cleaned up by the
tests.
