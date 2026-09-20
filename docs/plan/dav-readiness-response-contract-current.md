# DAV readiness and health response contract

Date: 2026-09-20

This receipt covers provider-free, offline `App::fake` state only. It did not
use Anytype, credentials, Android, DAVx5, Tasks.org, external clients or
network access, the personal Flatpak, or the pending credential artifact.

## Reproduced defects and focused fixes

1. `/health`, `/status`, and `/ready` accepted `POST` and fell through to the
   DAV handler, which returned a successful health response. The endpoints now
   accept `GET` only and return bounded `405` responses with `Allow: GET`.
2. Health responses had no cache prohibition, allowing an intermediary to
   retain a stale healthy result after a service failure. Health/readiness
   responses now carry `Cache-Control: no-store, no-cache, max-age=0`,
   `Pragma: no-cache`, and `X-Content-Type-Options: nosniff`.
3. A required audit failure was visible only in the nested audit object while
   the top-level health result still said `status=ok, ready=true`. The top-level
   result now reports `status=unavailable, ready=false`; optional degraded and
   recovering states remain visible while preserving readiness according to the
   configured policy.

## Local response matrix

| State/request | Status | Content type | Cache behavior | Body contract |
|---|---:|---|---|---|
| `GET /health`, healthy | 200 | JSON UTF-8 | no-store/no-cache | bounded `healthy`, `ready=true` |
| `GET /ready`, healthy | 200 | JSON UTF-8 | no-store/no-cache | bounded `healthy`, `ready=true` |
| `GET /status`, healthy | 200 | JSON UTF-8 | no-store/no-cache | same bounded status schema |
| Required audit unavailable, `GET /health` | 200 | JSON UTF-8 | no-store/no-cache | `unavailable`, `ready=false` |
| Required audit unavailable, `GET /ready` | 503 | JSON UTF-8 | no-store/no-cache | `unavailable`, `ready=false` |
| Optional audit degraded/recovering | 200 | JSON UTF-8 | no-store/no-cache | state is reported; readiness remains true |
| `POST` to any health path | 405 | text UTF-8 | no-store | bounded body and `Allow: GET` |

The response body does not echo request identifiers, endpoint values, space
identifiers, or credentials. Incoming request correlation remains available in
the bounded in-memory/audit event path; long synthetic IDs are capped there.

## Verification

```text
env -u LD_PRELOAD cargo fmt --all
env -u LD_PRELOAD cargo test -p any-cal-app --offline --test app -- --nocapture
41 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=<temporary> \
  cargo clippy -p any-cal-app --all-targets --offline -j1 -- -D warnings
passed
```

Focused tests cover healthy headers and body state, long correlation input,
required-audit failure mapping, no-cache behavior, and unsupported methods.

## Boundaries

This receipt does not claim external HTTP-client behavior, service-manager
integration, retention policy, persistent telemetry, TLS deployment, live
Anytype synchronization, or device-client interoperability.
