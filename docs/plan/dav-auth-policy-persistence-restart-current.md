# DAV auth-policy persistence and restart receipt

Date: 2026-09-20  
Scope: local Rust tests with synthetic credentials only.

## Result

The DAV identity seam now has an explicit, digest-only snapshot format and an
explicit service attachment method:

- `IdentityStore::save_to` serializes credential metadata, revocation state,
  principals, collection grants, and resource grants; raw tokens are not
  representable in the file.
- publication is `write -> fsync -> rename -> parent fsync`, with mode `0600`.
- `IdentityStore::load_from` rejects symlinks, non-private files, malformed or
  truncated JSON, unsupported snapshot versions, invalid digest encodings,
  duplicate credentials, and invalid time windows before returning a store.
- `AppGeneric::with_identity_file` loads the complete snapshot and attaches it
  to the existing DAV authorization boundary with an explicit caller-supplied
  clock value.

Restart-equivalent loading preserves authenticated, not-yet-valid, expired,
revoked, principal-scoped, and ACL decisions. The middleware test proves that a
loaded policy authorizes a DAV write while a nonmatching token remains `401`.

## Verification

All commands were run with `LD_PRELOAD` unset and an isolated target directory:

```text
env -u LD_PRELOAD cargo fmt --all -- --check
passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-persistence-target \
  cargo test -p any-cal-app --test identity --test identity_middleware --offline
10 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-persistence-target \
  cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
passed
```

Focused tests verify private mode, absence of the synthetic raw token from the
serialized state, revocation/ACL recovery, truncated-state rejection,
unsupported-version rejection, permission-invalid rejection, and middleware
wiring. Temporary fixtures are removed by the tests. No network, Anytype,
Android, provider, DAV client, personal Flatpak, or pending credential
artifact was accessed.

## Boundary

This is not a production account lifecycle, retention, encryption-at-rest, or
TLS deployment policy. The snapshot format stores SHA-256 digests as bearer
credential verifiers; operational rotation and administrative lifecycle remain
separate work.
