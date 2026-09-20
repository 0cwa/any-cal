# DAV synthetic key rotation and revocation receipt

Date: 2026-09-20  
Scope: provider-free Rust tests with synthetic credential generations only.

## Result

The local identity seam now has executable coverage for an explicit additive
rotation policy:

1. Add a replacement digest-backed generation while the prior generation is
   still valid (an overlap window).
2. Persist the overlapping generations through the private atomic snapshot.
3. Revoke the old generation after cutover.
4. Persist and reload the snapshot; the old generation remains rejected while
   the replacement remains authorized.

The tests also verify duplicate-generation creation is rejected without
changing the active credential, unknown revocation is a safe no-op, and
replaying a revocation is idempotent. The middleware test exercises both
generations during overlap and then proves, after restart-equivalent reload,
that the old generation receives `401` while the replacement can still write.

No raw credential value is emitted by the tests or receipt. Existing identity
debug/snapshot assertions verify that raw synthetic values are absent from
debug output and persisted JSON; only SHA-256 digest metadata is persisted.

## Verification

All commands ran with `LD_PRELOAD` unset, offline, and with a disposable Cargo
target directory:

```text
env -u LD_PRELOAD cargo fmt --all -- --check
passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-rotation-target \
  cargo test -p any-cal-app --test identity --test identity_middleware --offline
12 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-rotation-target \
  cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
passed
```

The focused scope accessed no network, Anytype credentials, Anytype state,
Android/provider state, DAV client, personal Flatpak state, or pending
credential artifact. Temporary identity snapshots are removed by the tests.

## Boundary

This proves local digest-backed overlap, revocation, persistence, restart
recovery, duplicate handling, and middleware authorization only. It does not
define a production account-management API, retention/deletion policy,
encrypted credential storage, TLS deployment, external client behavior, or a
live multi-user service lifecycle.
