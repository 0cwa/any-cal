# Service recovery fault-injection receipt

Date: 2026-09-20

## Scope and safety

This run used only temporary synthetic checkpoint, backup, lock, and fake
repository state under the system temporary directory. It did not access the
Anytype credential artifact, any external network, Anytype, Android, DAVx5,
Tasks.org, or a personal Flatpak. Temporary fixtures were removed by the test
cleanup paths.

## Fault matrix

| Boundary | Exercise | Result |
| --- | --- | --- |
| Pre-write | Injected `CommitFault::BeforeWrite` during an observed snapshot publication. | Error returned; in-memory candidate was not published. |
| Partial write | Injected truncated temporary output. | Error returned; prior state remained recoverable. |
| Pre-rename | Injected `CommitFault::BeforeRename` with an existing checkpoint. | Error returned; prior state and lock behavior remained bounded. |
| Directory sync | Injected failure after rename and before parent-directory sync. | Error returned; in-memory candidate was not published. |
| Primary truncation with valid backup | Reopened from a valid `.bak`, then interrupted the next commit. | Fixed: the valid backup is preserved and restart recovers the prior state. |
| Primary and backup corruption | Opened with both copies invalid, then retried. | Fixed: failed open removes its newly created lock, so the retry reports corruption rather than a stale-lock error. |
| Lock contention | Opened the same checkpoint concurrently. | Second writer is rejected; the first writer remains authoritative. |
| Restart | Dropped and reopened after pending enqueue and archive/tombstone updates. | Pending state, generation, and tombstones survive restart. |
| Resurrection | Reintroduced a tombstoned resource through ordinary snapshot replacement. | Rejected unless explicit resurrection API is used. |
| Artifact restore | Backed up and restored a synthetic artifact. | Atomic restore and redacted recovery events pass existing app tests. |

## Focused validation

`cargo fmt --check` passed after the change. Before the change, the existing
sync unit suite passed 14/14 tests and the app integration suite passed 23/23
tests. After the change, the environment's `rustc` began aborting immediately
with `fatal allocator error: invalid uninitialized allocator usage`, including
for `rustc --version`; therefore the updated Rust tests could not be compiled
in this run. This is an environment/toolchain blocker, not a test failure.

The new regression tests are:

- `recovery_does_not_overwrite_valid_backup_before_republish`
- `unrecoverable_corruption_does_not_leave_a_stale_lock`

## Implementation change

`SyncStore` now records whether it opened from the backup. Until a successful
primary publication, commit does not copy the unreadable primary over the
validated backup. Failed `open()` validation and unrecoverable corruption also
remove the lock created by that attempt.

## Residual gaps

- Re-run `cargo test -p any-cal-sync --lib`, `cargo test -p
  any-cal-observability --lib`, and the app recovery tests after the Rust toolchain
  allocator failure is resolved.
- Filesystem behavior on power loss and platform-specific lock semantics still
  require platform matrix testing; this receipt proves only deterministic local
  fault injection.
- No claim is made about live Anytype conflict semantics or external backup
  retention/encryption policy.
