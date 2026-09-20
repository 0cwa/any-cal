# DAV auth audit health service wiring — current evidence

Date: 2026-09-20

This lane was completed locally and provider-free. It did not access the
network, Anytype credentials, Android/provider state, DAVx5/Tasks.org, the
personal Flatpak, or the pending credential artifact.

## Implemented contract

`AppConfig` now supports an optional private audit directory, bounded queue
capacity, audit-health visibility, and an explicit `audit_required` policy.
The options are available through configuration files, environment variables,
and CLI flags. Invalid queue sizes, empty audit paths, and required-audit
without a directory fail closed before startup. Initialization failures are
reported only as a stable class (`permission`, `corrupt`, `locked`, `missing`,
or `io`), never as a path or raw I/O message.

When configured, `AppGeneric` owns the existing single-writer
`AuditEventWriter`. DAV request events are already redacted by the
observability layer before they are appended. With `audit_required=true`, an
append failure changes the response to a generic `503 audit unavailable` and
removes any ETag. With the default optional policy, request serving continues
while health exposes the audit fault when visibility is enabled.

`/health` and `/status` retain their existing bounded fields. When
`expose_audit_health=true`, they additionally expose only:

`enabled`, `state` (`healthy`, `degraded`, `unavailable`, or one-shot
`recovering`), `running`, queue counters/capacity, accepted/persisted/failed
counters, checkpoint/export readiness, and a stable last-failure class.

`/ready` returns `200` only when required audit is healthy and running; it
returns a generic `503` body otherwise. Audit fields are omitted entirely when
visibility is disabled. No endpoint includes audit paths, credentials,
resource IDs, or raw I/O text.

## Synthetic verification

Focused local checks, all with `LD_PRELOAD` unset and offline dependencies:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-service cargo fmt --all
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-service cargo test -p any-cal-app --test app --offline -j1
  34 passed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-service cargo test -p any-cal-app --test identity_middleware --offline -j1
  8 passed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-service cargo test -p any-cal-observability --offline -j1
  24 passed; 0 failed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-service cargo clippy -p any-cal-app --all-targets --offline -j1 -- -D warnings
  passed
```

The service tests cover healthy readiness, hidden audit fields, malformed
journal causing an unavailable state and fail-closed response, readiness
`503`, journal repair, and a deterministic recovering health observation.

## Boundary

This wiring does not add retention/deletion policy, persistent telemetry
backends, alerting/SLOs, external health exporters, or a claim that optional
audit mode makes every DAV write durable. Those remain explicit deployment
policy decisions.
