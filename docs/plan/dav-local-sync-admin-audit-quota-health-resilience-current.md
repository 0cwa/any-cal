# Local sync-admin audit quota and health resilience

Status: complete for the provider-free bounded scope on 2026-09-20.

## Scope and safety boundary

This lane used only in-process Rust tests and disposable temporary directories. It did not use the personal Flatpak, live Anytype, external clients or network, Android/provider state, DAVx5/Tasks.org, real credentials, or the pending credential-bearing artifact. No automatic retention policy was introduced.

`LD_PRELOAD` was explicitly unset for every quality-gate command. Temporary fixtures were created under the test process temporary directory and removed by the tests.

## Evidence

Commands:

```text
env -u LD_PRELOAD cargo test -p any-cal-observability --lib
env -u LD_PRELOAD cargo test -p any-cal-app --test app
env -u LD_PRELOAD cargo fmt --all -- --check
env -u LD_PRELOAD cargo clippy -p any-cal-observability --all-targets -- -D warnings
```

Results:

- `any-cal-observability`: 26 passed, 0 failed.
- `any-cal-app` app integration suite: 45 passed, 0 failed.
- Rust formatting check passed.
- Observability Clippy with `-D warnings` passed.

## Acceptance findings

### Bounded quota/backpressure

- Audit readback accepts only `limit` in `1..=64`; zero, over-limit, duplicate, unsupported, malformed, and body-bearing requests return bounded 400 responses.
- Cursor/readback output is a bounded summary projection. Stored messages, resource identifiers, credentials, and paths are not returned.
- The writer uses a single process-exclusive worker and a bounded synchronous queue. The concurrent producer test uses queue capacity 2 and 128 events; sequences are exactly `1..=128`, the queue drains to zero, and no event is duplicated or lost.
- Event messages and reports have fixed bounds. The stress test confirms redaction remains effective under concurrent production.

### Health, worker failure, and recovery

- Corrupt audit data produces stable `Corrupt` classification and degraded/unavailable state without exposing the underlying path or parse text.
- Required audit mode returns 503 for DAV activity and `/ready` while the journal is unavailable; `/health` reports `status=unavailable` and `ready=false`.
- After the corrupt fixture is repaired, the next health observation reports `state=recovering`; readiness and normal operation recover without changing the response contract.
- Writer lock contention is classified as `Locked`; stale-lock recovery is explicit and exact-path only. A fresh writer reopens with preserved sequence/checkpoint state.
- Optional audit degradation does not incorrectly make the service unready; required mode fails closed. This distinction is covered by app integration tests.
- Export/readback corruption is reported as a bounded 503/diagnostic state; no raw malformed content is reflected.

### Correlation and redaction

- Admin responses carry bounded correlation headers where the request ID passes validation.
- Readback exposes only operation/status/category/timestamp/sequence and a correlation digest.
- Health JSON exposes queue counters, state, readiness, and stable failure classes only when configured. Space IDs, credentials, paths, raw messages, and malformed input are absent.

## Defect disposition

No code defect was reproduced in this bounded lane. Existing implementation and tests cover the requested local quota, pressure, worker-failure, truthful-health, and recovery behavior. Live load, real external service behavior, automatic retention, and Android client interoperability remain outside this unit by design.

