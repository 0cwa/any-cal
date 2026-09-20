# DAV readiness/recovery/configuration integration

Date: 2026-09-20

This receipt covers provider-free local synthetic state only. It does not use
Anytype, credentials, Android, DAVx5, Tasks.org, external clients/network,
the personal Flatpak, or the pending credential artifact.

## Verification

The focused integration suite passes:

```
env -u LD_PRELOAD cargo test -p any-cal-app --offline --test app -- --nocapture
38 passed, 0 failed
```

The audit-store unit suite passes:

```
env -u LD_PRELOAD cargo test -p any-cal-observability --offline --lib
24 passed, 0 failed
```

Formatting and clippy pass. Clippy was run in a fresh temporary target
directory because the shared target contained artifacts from a different
rustc minor version:

```
env -u LD_PRELOAD cargo fmt --all -- --check
env -u LD_PRELOAD CARGO_TARGET_DIR=<temporary> cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
```

## Lifecycle matrix

- Required audit configuration without an audit directory fails before app
  construction with a bounded configuration error.
- Optional audit mode starts and reports ready without an audit writer.
- A live synthetic writer lock is refused; an exact-path lock containing a
  non-live synthetic PID is removed only through explicit stale-lock recovery.
- Graceful app drop releases the writer lock; restart reopens the same audit
  journal and remains ready.
- Required audit corruption transitions health to `unavailable`, returns 503
  for readiness and durable requests, and recovers through a successful
  append after the corrupt active file is removed.
- Optional audit corruption keeps `/ready` at 200 while health reports the
  degraded/unavailable audit state; after repair, the next successful append
  transitions through `recovering` and back to healthy.
- A DAV write persists the sync checkpoint before the audit event is exported;
  both artifacts survive restart and the audit writer's lock is absent after
  graceful shutdown.
- Health/status output remains bounded and excludes synthetic space/path
  identifiers and credential-like values.

## Reproduced defect and fix

The optional-policy recovery test initially returned HTTP 503 from `/ready`
after audit degradation. `AppGeneric::handle` unconditionally used audit
health as a readiness gate whenever an audit writer existed, ignoring
`audit_required=false`. The readiness calculation now gates on audit health
only when `audit_required` is true; optional audit degradation remains visible
through health/status without taking the service out of readiness.

No other production defect was reproduced in this unit.

## Boundaries

This receipt does not claim automatic retention/deletion, production service
manager integration, persistent telemetry, live remote synchronization, or
device-client interoperability.
