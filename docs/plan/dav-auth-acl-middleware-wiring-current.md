# DAV identity and ACL middleware wiring

Date: 2026-09-20  
Scope: local Rust request-path tests with synthetic principals and
nonsecret fixture tokens only.

## Result

The app request path now supports an explicitly attached in-memory
`IdentityStore` and synthetic clock. For DAV requests, authentication and
ACL evaluation run before the DAV repository handler. Collection and resource
scopes are derived only from the configured CardDAV/CalDAV collection names;
unknown paths are never used to select a policy scope.

The middleware enforces:

- malformed, missing, duplicate, invalid, expired, and revoked credentials
  return `401` with the normal bounded challenge;
- collection policy denials return `403`;
- resource-level denials return `404` with a generic body, preventing resource
  existence and identifier disclosure;
- credential capability restrictions are checked in addition to principal ACLs;
- resource-level grants work without granting an entire collection; and
- health/status continue to use the existing local/global health credential
  boundary rather than being silently exposed through a DAV identity token.

The identity attachment is intentionally explicit and in-memory. Persistent
credential storage, administrative revocation, clock/skew policy, and
production account lifecycle remain separate work.

## Validation

All commands ran with `LD_PRELOAD` unset, offline, and with a disposable Cargo
target directory:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-middleware-target \
  cargo test -p any-cal-app --test identity_middleware --offline
3 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-middleware-target \
  cargo test -p any-cal-app --test identity --offline
4 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-middleware-target \
  cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-middleware-target \
  cargo fmt --all -- --check
passed
```

The focused middleware tests cover request ordering, collection discovery,
resource hiding, malformed/duplicate headers, expiry, revocation, capability
scope, and denied-write non-mutation. No external network, Anytype state,
Android/provider state, personal Flatpak state, or pending credential artifact
was accessed.

## Boundary

This receipt does not claim persistent multi-user deployment, TLS termination,
DAVx5/Tasks.org interoperability, or live Anytype validation. Those remain
authority-gated lanes.
