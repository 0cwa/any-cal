# DAV auth audit health and failure semantics — current evidence

Date: 2026-09-20

This lane stayed local and provider-free. It did not read the pending
workspace credential artifact, use Anytype/Android/client state, access the
network, or launch the personal Flatpak.

## Reproduced boundary and fix

`AuditEventWriter` previously exposed only raw `io::Result` values. A caller
could not distinguish a corrupt journal from a temporarily unavailable store,
or report whether the writer had recovered, without retaining filesystem
error text (which can contain paths).

The observability crate now exposes bounded, serializable state:

- `AuditHealthState`: `Healthy`, `Degraded`, or `Unavailable`.
- `AuditFailureClass`: `Permission`, `Corrupt`, `Locked`, `Missing`, `Io`, or
  `Queue`.
- `AuditOperation` and `classify_audit_failure`, which classify only the
  `io::ErrorKind`; raw paths and messages are not retained.

Append failure transitions the writer to `Unavailable`; export/checkpoint
failure transitions it to `Degraded`. A successful append, export, or
checkpoint clears the prior failure and returns the writer to `Healthy`.
Queue-send failure is classified as `Queue`. Existing bounded queue behavior
and single-writer ownership are unchanged.

These results give a request layer a fail-closed decision: an operation that
cannot meet its required audit guarantee receives the original error and no
successful persistence count is reported. This lane does not claim that every
DAV request is currently configured to require durable audit persistence; that
policy remains an integration decision.

## Synthetic fault matrix

| Fault | Observed result | Safe classification/status |
|---|---|---|
| Store path is a regular file (unavailable/read-only stand-in) | writer open fails before ownership is claimed | `Io`; no writer is exposed |
| Existing writer lock | second writer open fails | `Locked`; first writer remains owner |
| Interior malformed journal record | export fails; no records are silently rewritten | `Corrupt` / `Degraded` |
| Corrupt journal repaired | export succeeds and returns the original record count | `Healthy`, prior failure cleared |
| Permission error classification | synthetic `PermissionDenied` maps deterministically | `Permission` |
| Bounded queue | existing concurrent producer test remains green | bounded backpressure; queue failures are `Queue` |

The read-only filesystem case was represented with a regular-file store path
because the test process is privileged enough that chmod-based denial is not a
portable assertion. Permission classification itself is covered with a
synthetic `PermissionDenied` error.

## Verification

Commands, all with `LD_PRELOAD` unset and offline dependencies:

```text
env -u LD_PRELOAD cargo fmt --all
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-target cargo test -p any-cal-observability --offline
  24 passed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-target cargo clippy -p any-cal-observability --all-targets --offline -- -D warnings
  passed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-target cargo test -p any-cal-app --test app --offline
  32 passed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-target cargo test -p any-cal-app --test identity_middleware --offline
  8 passed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-health-target cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
  passed
```

No test output contains real credentials. The new health fields contain only
enum values, counters, and a last failure class; no path, token, resource
identifier, or raw I/O message is serialized.

## Remaining boundary

This work does not introduce automatic retention/deletion, persistent
telemetry, production alerting, or a global policy that makes all DAV writes
depend on audit durability. A later service-integration unit must choose and
document whether audit persistence is mandatory for a given request class and
wire the health state into the external `/health` contract.
