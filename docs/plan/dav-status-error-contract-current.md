# DAV status/error contract — current

Date: 2026-09-20
Scope: provider-free synthetic requests only. No external network, credentials,
Anytype, Android, DAVx5/Tasks.org, personal Flatpak, or pending credential
artifact was accessed.

## Result

The local DAV handler now applies one response contract after every handler
branch:

- status mappings remain typed: malformed/invalid input `400`, unsupported
  method `405`, missing resource `404`, timeout `408`, duplicate/conflict
  `409`, stale precondition `412`, unsupported media `415`, and unavailable
  service `503`;
- error responses receive bounded `text/plain; charset=utf-8` content when a
  branch did not provide a content type, `Cache-Control: no-store`, and a
  generic body when the branch returned no body;
- `Allow` is added to `405` responses when absent, scoped to the DAV path;
- a syntactically safe, bounded `X-Request-ID` is echoed for correlation;
- body text never includes the requested resource path or credential-like
  input. Oversized, whitespace-containing, control-containing, or otherwise
  unsafe request IDs are not echoed.

## Evidence

Focused commands, with `LD_PRELOAD` unset:

```text
cargo fmt --all -- --check                         PASS
cargo test -p any-cal-dav-server --tests -- --nocapture  PASS: 46 tests
cargo clippy -p any-cal-dav-server --all-targets -- -D warnings PASS
```

The added synthetic matrix covers unsupported methods, missing resources,
unsupported media, malformed REPORT input, repository timeout, and stale
preconditions. It asserts status, content type, cache policy, Allow metadata,
correlation, bounded bodies, and path/resource non-leakage.

## Narrow defect fixed

Previously, early-return DAV error branches could omit `Content-Type`, cache
policy, `Allow`, correlation, or any body. The outer finalization seam removes
that branch-order dependence without exposing repository details. Existing
success response media types and DAV XML bodies remain unchanged.

## Remaining boundary

Authentication/ACL denial responses and health/readiness responses are owned by
the application layer and were not changed in this lane. External client
compatibility and live service behavior remain separate work units.
