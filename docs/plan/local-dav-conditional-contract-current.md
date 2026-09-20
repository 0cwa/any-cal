# Local DAV conditional-contract regression

Date: 2026-09-20

This receipt records a local-only, synthetic regression run. It did not access
Anytype, Android, DAVx5, Tasks.org, external network state, the personal
Flatpak, or the pending workspace credential artifact. No credential-bearing
file was read.

## Reproduced defect and focused fix

The in-process DAV matrix reproduced incorrect handling of the HTTP
`If-Match: *` wildcard:

- PUT treated the wildcard as an ETag mismatch and returned `412`.
- DELETE mapped the wildcard to `If-None-Match`, also returning `412` for an
  existing resource.

The DAV adapter now treats the wildcard as matching an existing representation
for both update and archive/delete. A narrow regression test covers successful
wildcard update, read-after-update, wildcard delete, and final `404`.

## Coverage

The focused DAV-server suite passed 19 tests:

- CardDAV and CalDAV discovery, slash-terminated homes, proxy-origin and href
  validation;
- GET/PUT/DELETE lifecycle, stable ETags, `If-Match`, `If-None-Match`, and
  rejected stale writes without mutation;
- collection query and multiget filtering, requested-property selection,
  per-href `404` multistatus entries, XML/content-type handling, malformed
  report rejection, and bounded failure mapping;
- VTODO timezone forms, capability advertisement, and replay determinism;
- HTTP framing/parser tests, including sequential request boundaries,
  malformed lengths, unsupported transfer encoding, and connection semantics.

The application integration suite also passed 28 tests, including restart,
stable ETag/archive behavior, actual CardDAV/CalDAV seams, checkpoint reopen,
failure mapping, and LAN policy boundaries.

## Verification commands

All commands were run with `LD_PRELOAD` unset and offline:

```text
env -u LD_PRELOAD cargo fmt --all
env -u LD_PRELOAD cargo test -p any-cal-dav-server --tests --offline
env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --all-targets --offline -- -D warnings
env -u LD_PRELOAD cargo test -p any-cal-app --test app --offline
```

Results: formatting clean, 19 DAV tests passed, clippy passed with
`-D warnings`, and 28 application integration tests passed.

No broader protocol refactor or external interoperability claim is made by
this receipt.
