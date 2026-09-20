# CardDAV vCard projection and merge-conflict receipt

Date: 2026-09-20
Scope: local/offline synthetic fixtures only.

## Result

The local projection and repository behavior satisfies the bounded lane:

- Updating only an addressed projection property preserves omitted repeated and
  opaque properties.
- Explicitly addressing a repeated property replaces that complete property
  group deterministically, preserving occurrence order and labels without
  duplication.
- UID, Anytype object identity, resource identity, revision, and ETag remain
  stable across a repository clone/reopen boundary.
- A stale `If-Match` condition returns a typed precondition conflict and leaves
  the newer representation unchanged.
- The application emits a bounded `dav.request` conflict event for the stale
  write. The event contains only the HTTP status and synthetic correlation
  identifier; the vCard body and identifiers are not included.

No behavioral defect was reproduced, so no production implementation change
was made. The lane added focused regression coverage in:

- `crates/core/tests/roundtrip.rs`
- `crates/core/tests/repository.rs`
- `crates/app/tests/app.rs`

## Verification

All commands were run with `LD_PRELOAD` unset and a clean temporary target
directory (`/tmp/any-cal-carddav-target`):

- Core repository tests: 7 passed.
- Core vCard round-trip/projection tests: 8 passed.
- Application stale-conflict receipt test: 1 passed.
- `cargo fmt --check`: passed.
- `cargo clippy -p any-cal-core -p any-cal-app --tests -- -D warnings`:
  passed.

The pre-existing target directory contained artifacts built by another Rust
toolchain; verification used the isolated temporary target rather than
deleting shared build output.

## Boundaries

This receipt does not claim behavior for external CardDAV clients, DAVx5,
Tasks.org, Anytype, Android providers, network transport, or retention policy.
No credentials, pending credential-bearing artifacts, personal Flatpak state,
or external state were accessed.
