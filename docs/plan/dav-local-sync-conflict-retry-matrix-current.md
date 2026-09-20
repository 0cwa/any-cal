# Local DAV sync conflict/retry matrix

Date: 2026-09-20

## Scope and safety

This was a provider-free, offline validation lane. It used only synthetic
DAV requests, the in-memory fake Anytype transport, and disposable checkpoint
files created by the tests. It did not access the pending credential artifact,
Anytype accounts or credentials, external services, Android/provider state,
DAVx5, Tasks.org, or the personal Flatpak. No credential values, raw live
payloads, or live identifiers were retained.

## Matrix result

| Scenario | Expected invariant | Result |
| --- | --- | --- |
| Stale `If-Match` update/delete | Return `412`; current DAV body and ETag remain unchanged | pass |
| Remote drift while local write is pending | Classify as conflict; documented later-write-wins policy reports remote change without claiming optimistic concurrency | pass |
| Not-sent transient failure | Retry only while the bounded attempt limit allows it | pass |
| Timeout after mutation commit | Reconcile by read before any retry; do not duplicate the mutation | pass |
| Same-object concurrent writes | Per-object lock serializes the critical section | pass |
| Archive/delete | Remove from normal observation and retain an identity tombstone | pass |
| Tombstone reappearance | Ordinary refresh rejects resurrection; explicit resurrection is required | pass |
| Checkpoint/restart | Pending state and tombstones survive reopen; corrupt primary recovers from a valid backup | pass |
| Candidate checkpoint failure | Failed write, rename, or directory sync does not publish candidate state | pass |
| Diagnostic safety | Conflict and retry receipts contain bounded categories/statuses and no body, token, or full identity | pass |

## Verification evidence

Focused commands used the pinned Rust toolchain, `LD_PRELOAD` unset, and Cargo
offline:

```text
cargo test -p any-cal-app --test local_sync_mutation --offline -- --test-threads=1
  2 passed
cargo test -p any-cal-app --test app --offline -- --test-threads=1
  41 passed
cargo test -p any-cal-sync --lib --offline -- --test-threads=1
  17 passed
cargo test -p any-cal-anytype-adapter --offline -- --test-threads=1
  16 adapter, 15 HTTP, and 4 wire tests passed; 1 socket TLS test ignored by scope
cargo fmt --all -- --check
  passed
cargo clippy -p any-cal-sync -p any-cal-anytype-adapter -p any-cal-app --tests \
  --offline -- -D warnings
  passed
```

The full `any-cal-app` unit-test target still contains four host-socket tests
that cannot bind sockets in the restricted runner; those are environment
permission failures, not this local matrix. The app integration target above
passes and exercises the DAV seam without host sockets.

## Change disposition and boundary

No local behavioral defect was reproduced, so no production code change was
needed. Existing focused tests cover the matrix, including deterministic
timeout-after-commit reconciliation and concurrent per-object serialization.
These results establish local bookkeeping and fake-transport safety only. They
do not establish server-side ETag/revision, rate-limit, distributed conflict,
or live Anytype semantics; those remain separate authority-gated validation.
