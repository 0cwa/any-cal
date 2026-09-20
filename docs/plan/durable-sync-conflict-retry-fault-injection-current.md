# Durable sync conflict/retry fault-injection receipt

Date: 2026-09-20

## Scope and safety

This lane was local and offline. It used only the repository's fake Anytype
transport and temporary checkpoint paths created by the existing unit tests.
It did not access the workspace credential artifact, external network,
Anytype, Android/provider state, DAVx5, Tasks.org, or a personal Flatpak.
No credential values, raw payloads, or live identifiers were retained.

## Fault matrix

| Boundary | Deterministic coverage | Result |
| --- | --- | --- |
| Stale remote drift with pending write | `classify` reports `Conflict`; explicit policy reports later remote change without claiming optimistic concurrency | pass |
| Unknown mutation outcome | Fake timeout-after-create commits once, then bounded read reconciliation proves the mutation; no blind retry | pass |
| Retry classification | Unknown outcome requires reconciliation; not-sent transient retries once up to `MAX_RETRY_ATTEMPTS`; exhausted/non-transient work stops | pass |
| Per-object serialization | Eight concurrent synthetic operations on one object observed a maximum active section of one | pass |
| Tombstone/archive | Archive removes observed state and records identity tombstone; normal refresh cannot resurrect it | pass |
| Explicit resurrection | Only the explicit resurrection API removes the tombstone and republishes the resource | pass |
| Checkpoint publication | Pre-write, partial-write, pre-rename, and directory-sync faults leave candidate state unpublished | pass |
| Restart/recovery | Pending operations survive reopen; corrupt primary recovers from backup; unrecoverable corruption removes the lock; valid backup is preserved until republish | pass |

## Verification

All Rust commands used the installed pinned rustup toolchain through
`PATH=/home/x/.cargo/bin:/usr/bin:/bin`, `LD_PRELOAD` unset, and Cargo offline:

```text
cargo test -p any-cal-anytype-adapter --offline       16 passed, 1 ignored
cargo test -p any-cal-sync --lib --offline            16 passed
cargo fmt --all -- --check                            passed
cargo clippy -p any-cal-sync -p any-cal-anytype-adapter --all-targets --offline -- -D warnings
                                                       passed
```

The one ignored adapter test is the existing host-socket TLS test; this lane
does not authorize socket or live-service validation.

## Change disposition

No behavioral defect was reproduced, so no production implementation change
was made. A focused adapter test was added to prove that the existing
per-object lock cannot overlap across concurrent callers. The existing retry,
reconciliation, conflict-policy, tombstone, checkpoint, and restart behavior
remains unchanged.

## Boundary

These tests establish local bookkeeping and fake-transport safety only. They
do not prove server-side ETag/revision, rate-limit, or distributed conflict
semantics. Those remain dependent on a supported live Anytype sink and an
explicitly authorized external validation lane.

