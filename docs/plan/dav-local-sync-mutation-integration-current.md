# Local DAV → sync mutation integration

Date: 2026-09-20

This receipt covers only provider-free, offline fixtures. It does not claim
live Anytype, external DAV client, Android, DAVx5, Tasks.org, or personal
Flatpak interoperability.

## Result

The focused matrix passes:

```text
cargo test -p any-cal-anytype-adapter --test adapter       16 passed
cargo test -p any-cal-app --test local_sync_mutation        2 passed
cargo test -p any-cal-app --test app                       41 passed
cargo clippy -p any-cal-anytype-adapter -p any-cal-app --tests -- -D warnings  passed
```

The checks used Rust 1.97.1 with `LD_PRELOAD` unset and an isolated temporary
Cargo target directory for the final Clippy run.

## Covered behavior

- DAV PUT create and conditional update produce a checkpointed observed row;
- stale `If-Match` PUT is rejected without changing the representation,
  ETag, checkpoint generation, or remote fake object;
- stale `If-Match` DELETE is rejected before archive and does not remove the
  visible representation;
- a successful DELETE archives the resource, removes it from the observed
  snapshot, and records a tombstone;
- reopening the application preserves the tombstone and the archived remote
  resource remains absent from normal DAV GET;
- duplicate pending-operation enqueue is idempotent;
- pending drift is classified as a conflict, while the explicit Anytype
  later-write-wins policy is separately recorded;
- complete observation omission creates a tombstone, ordinary replay refuses
  resurrection, and an explicit resurrection path clears that tombstone;
- redacted event messages do not contain the synthetic resource identifier.

## Defect fixed

`AnytypeRepository` previously performed the remote create/update/archive/delete
before applying the local repository precondition. A stale DAV update could
therefore return `412` only after the remote representation had already been
changed. The adapter now runs the local envelope, identity, and conditional
write checks against a cloned cache before each remote mutation. The remote
operation is attempted only after that preflight succeeds.

This preserves the observed Anytype later-write-wins behavior for accepted
writes while keeping the local DAV ETag/precondition contract atomic on
rejected writes.

## Boundaries and follow-up

There is no DAV restore/unarchive method in the current protocol profile;
restart recovery and the explicit sync-store resurrection API are covered,
while external restore semantics remain a separate design decision. The
receipt does not establish server-side Anytype ETag support or external-client
compatibility.
