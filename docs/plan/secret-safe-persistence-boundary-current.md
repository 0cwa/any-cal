# Secret-safe persistence boundary — current receipt

Date: 2026-09-20
Scope: offline workspace-only validation with synthetic sentinels. The pending
`.local/anytype-test-v2/credentials.env` artifact was not opened, moved, hashed,
or otherwise accessed.

## Result

The bounded matrix found and fixed two local boundary defects:

1. `App::check()` and `/health` previously emitted configured space and
   collection values verbatim. They now expose only `*_configured` booleans.
2. Atomic backup/restore artifacts, sync checkpoints, and the sync writer lock
   now receive explicit Unix mode `0600` after creation/publication.

Canonical sync checkpoints continue to contain the identities required for
reconciliation. They are private recovery state, not diagnostic/report output;
the diagnostic surface does not echo those values.

## Synthetic matrix

Inputs included credential-shaped bearer/token values, Anytype-object-shaped
identifiers, collection-shaped identifiers, malformed configuration text,
oversized diagnostic messages, corrupt primary checkpoints, injected partial
writes, pre-rename failures, directory-sync failures, and symlink artifact
paths. Values were asserted only inside tests and are absent from this receipt.

Observed guarantees:

- `/health`, `/status`, `App::check()`, error diagnostics, and bounded events do
  not contain the synthetic bearer or raw identifier sentinels.
- Diagnostic messages remain bounded to `MAX_MESSAGE`.
- Event reports remain bounded to `MAX_REPORT_EVENTS`; retained events remain
  bounded to `MAX_RETAINED_EVENTS`.
- Atomic backup/restore publishes complete files and refuses symlink paths.
- Sync checkpoint commit faults do not advance in-memory state; restart recovers
  from a valid backup without overwriting that backup prematurely.
- Primary checkpoint, backup, temporary publication, and writer lock files are
  mode `0600` on Unix.
- Restart preserves pending operations and tombstone safety.
- Temporary test artifacts were removed after the run.

## Verification

All commands ran with `LD_PRELOAD` unset and without network, Anytype, Android,
DAVx5/Tasks.org, personal Flatpak, or external credential access:

- `cargo fmt --all -- --check`
- `cargo test -p any-cal-observability --lib` — 13 passed
- `cargo test -p any-cal-sync --lib` — 17 passed
- `cargo test -p any-cal-app --test app` — 28 passed
- `cargo clippy -p any-cal-app -p any-cal-observability -p any-cal-sync --all-targets -- -D warnings` — passed in an isolated temporary target directory

The focused regression adds one app diagnostic test and one sync artifact-mode
test; the observability backup/restore test now checks private modes.
