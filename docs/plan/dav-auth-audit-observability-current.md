# DAV authentication audit and observability — current evidence

Date: 2026-09-20

## Scope

This receipt covers only local Rust tests with synthetic identity records and
synthetic DAV requests. It did not read or modify the pending credential
artifact, use Anytype credentials, contact an external client or network, use
Android/provider state, or use the personal Flatpak.

Persistent telemetry, retention policy, production metrics, and real
credential values remain out of scope.

## Reproduced gap and focused change

Before this run, identity middleware returned the correct HTTP denial but did
not emit an authentication lifecycle event. This made local expiry, invalid,
and authorization outcomes indistinguishable in the in-memory diagnostic
buffer.

`AppConfig.emit_auth_events` now controls a bounded `dav.auth` event stream.
The setting is disabled by default and can be selected through configuration
or `ANY_CAL_EMIT_AUTH_EVENTS`. Events contain only:

- a bounded request correlation value;
- outcome (`missing`, `invalid`, `not_yet_valid`, `expired`, `revoked`,
  `authenticated`, `forbidden`, or `not_found`);
- coarse scope (`contacts`, `tasks`, or `other`); and
- `principal_class=known|unknown`.

Credential material, credential IDs/generation handles, raw principals,
resource paths/IDs, configured Space/collection IDs, and request bodies are
not emitted. Event messages continue through the observability redaction and
length bounds, and failed outcomes are classified as `Auth`.

## Evidence

Commands, with dynamic loader injection explicitly unset:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-target cargo test -p any-cal-app --test identity_middleware --offline
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-target cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
env -u LD_PRELOAD cargo fmt --all -- --check
```

Results:

- 8 identity-middleware tests passed;
- opt-in audit test confirmed missing, denied, and successful synthetic
  requests produce exactly three bounded events;
- disabled-by-default test confirmed no `dav.auth` event is retained;
- audit messages did not contain synthetic bearer values, resource paths, or
  configured identifiers;
- correlation retained the supplied bounded request ID;
- clippy passed with warnings denied;
- formatting passed.

Existing identity/rotation and observability tests remain the supporting
evidence for expiry, revocation, restart, redaction, cardinality, and bounded
event storage. This receipt does not claim persistent telemetry, event
retention/rotation, or production account lifecycle behavior.

## Remaining boundary

Generation handles are intentionally not exposed in events. If operators
later require generation-level audit correlation, add a separately designed
opaque, non-reversible handle with an explicit privacy review; do not expose
credential IDs or token-derived values directly.
