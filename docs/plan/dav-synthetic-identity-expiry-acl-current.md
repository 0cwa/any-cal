# Synthetic DAV identity, expiry, and ACL receipt

Date: 2026-09-20  
Scope: local synthetic fixtures and offline Rust tests only.

## Result

The identity policy seam now has deterministic coverage for:

- nonsecret principal and credential identifiers;
- valid, not-yet-valid, expired, revoked, and unmatched credentials;
- malformed control-character credentials failing closed;
- collection and resource-level read/write decisions for multiple principals;
- denied-resource responses represented as `NotFound`, avoiding existence
  disclosure;
- authenticated capability discovery, filtered by credential capability and
  never exposing resource identifiers; and
- restart-equivalent reconstruction by cloning the policy/credential snapshot.

Credential material is stored only as a SHA-256 digest in this synthetic
policy layer. The token is never included in `Debug` output, test output, or
this receipt. No external service, network client, Anytype state, Android
state, personal Flatpak, or pending credential artifact was accessed.

## Important boundary

`crates/app/src/identity.rs` is intentionally a policy seam and is not wired
into the current production HTTP entry point. The current entry point still
uses its existing single configured DAV credential. This lane therefore proves
the multi-principal semantics without claiming production account lifecycle,
persistent credential storage, retention, or TLS deployment.

## Verification

Commands were run with `LD_PRELOAD` unset and an isolated target directory:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-identity-target \
  cargo test -p any-cal-app --test identity --offline -- --nocapture
4 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-identity-target \
  cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
passed

env -u LD_PRELOAD cargo fmt --all
passed
```

The broader app test target was also attempted. Three pre-existing socket
tests cannot bind/listen in the restricted sandbox and fail with OS
`PermissionDenied`; the five non-socket app unit tests pass. This receipt does
not treat those environment failures as identity failures.

## Follow-up

Before wiring this seam into the service, add an explicitly reviewed
persistent credential format, clock source/skew policy, administrative
revocation lifecycle, and HTTP mapping for `401` versus resource-hiding
`404`. Those are outside this synthetic/offline lane.

