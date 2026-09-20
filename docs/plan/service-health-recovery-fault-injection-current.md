# Service health/recovery fault-injection receipt

Date: 2026-09-20
Scope: local, synthetic, offline only. No external network, Anytype
credentials, Android/provider state, DAVx5/Tasks.org state, personal Flatpak
state, or pending credential artifact was accessed.

## Matrix

| Fault | Observed status/category | Recovery implication |
| --- | --- | --- |
| Invalid configuration | `ConfigError::Invalid`/`Missing`; constructor fails before service start | Correct configuration and retry startup |
| Malformed vCard write | HTTP 400; event category `protocol` | Repair client payload; do not retry unchanged input |
| Synthetic repository timeout | HTTP 408; event/health category `timeout` | Bounded retry/reconciliation path |
| Checkpoint commit failure (`BeforeWrite`) | HTTP 500; event/health category `recovery`; in-memory checkpoint state not published | Preserve prior checkpoint and recover from the last valid state |
| Auth/forbidden/conflict | Existing HTTP mappings 401/403/409/412 with typed event categories | Reauthenticate, deny, or reconcile according to status |

The timeout and checkpoint rows exposed two local boundary defects. Repository
timeouts were previously collapsed to HTTP 500 and therefore classified as
`anytype`; they now map to HTTP 408. Checkpoint publication failure was
previously classified from HTTP 500 as `anytype`; the application now retains a
local `checkpoint_failed` marker and emits `recovery`.

## Bounded observability checks

- Fault-matrix test verifies protocol, timeout, and recovery categories through
  the running application seam.
- Correlation IDs remain attached to events; event retention is capped at
  `MAX_RETAINED_EVENTS` and reconciliation reports at `MAX_REPORT_EVENTS`.
- Existing redaction tests cover bearer/API-key, email, phone, address, and
  bounded-message handling. The new receipt contains no live identifiers or
  secret values.
- Checkpoint fault test verifies that the failed candidate is not published to
  the in-memory sync state.

## Verification

Passed:

```text
env -u LD_PRELOAD cargo fmt --all
env -u LD_PRELOAD cargo test -p any-cal-app --offline --test app  # 24 passed
env -u LD_PRELOAD cargo test -p any-cal-observability --offline   # 13 passed
CARGO_TARGET_DIR=/tmp/any-cal-fault-target env -u LD_PRELOAD \
  cargo clippy -p any-cal-app -p any-cal-dav-server --all-targets --offline -- -D warnings
```

The repository target directory contained artifacts built by a different
compiler, so clippy used a separate temporary target directory; this avoided
cleaning or mutating unrelated build state.

## Deferrals

This receipt proves deterministic local behavior only. It does not prove live
Anytype interoperability, production telemetry, backup-retention policy,
Android/provider behavior, or external client compatibility.
