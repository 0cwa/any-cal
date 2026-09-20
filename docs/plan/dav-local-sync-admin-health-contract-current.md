# Local DAV sync admin health contract

Date: 2026-09-20

This receipt covers provider-free `App::fake` requests with disposable
synthetic checkpoint state. It did not access Anytype, external clients or
network, Android/provider state, DAVx5/Tasks.org, the personal Flatpak, real
credentials, or the pending credential artifact. No retention policy was
introduced.

## Result

The local admin and health response contract is verified for the bounded
states exercised here:

- `GET /health`, `/status`, and `/ready` remain typed, uncached JSON
  responses. A configured local health credential authorizes all three, while
  the operator credential remains required for admin sync operations.
- `GET /admin/sync/capabilities` reports export/restore availability from the
  configured sync checkpoint; disabled sync reports both capabilities as
  false. `POST /admin/sync/export` and `/restore` use the fixed adjacent
  export artifact and bounded receipts.
- Admin errors for unsupported methods, non-empty request bodies, missing
  checkpoint state, malformed/tampered restore artifacts, and failed restore
  operations are bounded JSON errors with `no-store`, `Allow` where
  applicable, and no path, identifier, payload, or credential disclosure.
- Safe request IDs are returned as `X-Request-ID` on health and authenticated
  admin responses. Unsafe, oversized, whitespace, or control-containing IDs
  are not echoed. Unauthorized admin responses keep the generic denial body
  while adding only bounded cache and correlation metadata on the admin path.
- Existing sync export/restore tests prove valid round trips and unchanged
  active state after malformed, incompatible, tampered, truncated, or
  symlinked inputs. The current lane additionally verifies the admin-facing
  status and auth boundary around those operations.

## Reproduced defect and focused fixes

1. `/ready` was not treated as a health-only endpoint: a configured local
   health credential could access `/health` and `/status` but not readiness.
   Readiness now follows the same local-health authorization policy.
2. Health and admin responses did not consistently expose bounded request
   correlation. Health responses now return a safe `X-Request-ID`; admin
   capability, success, and error responses do likewise. Admin errors also
   carry `Cache-Control: no-store`.

No changes were made to generic DAV authorization/error headers, preserving
the established external DAV response contract.

## Verification

Commands ran with `LD_PRELOAD` unset and offline:

```text
env -u LD_PRELOAD cargo fmt --all -- --check                         PASS
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo test -p any-cal-app --test app --offline -- --nocapture      PASS: 43 passed; 0 failed
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo clippy -p any-cal-app --test app --offline -- -D warnings    PASS
```

The focused test `sync_admin_health_contract_is_correlated_and_separates_health_auth`
covers local-health versus operator authorization, readiness, capability
availability, malformed body, unsupported method, safe/unsafe correlation,
bounded status, and redaction. Existing app tests cover audit unavailable,
degraded/recovery, checkpoint failure, atomic restore, and service lifecycle.

## Explicit boundaries

This does not claim automatic retention or scheduled backups, persistent
production telemetry, encryption at rest, multi-user admin policy, live
Anytype synchronization, or external DAV client compatibility. Those remain
separate gated work.
