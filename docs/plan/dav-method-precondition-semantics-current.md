# Local DAV method and conditional-precondition semantics

Date: 2026-09-20

This is a provider-free synthetic receipt. It used only `MemoryRepository` and
the in-process DAV handler. It did not access Anytype, Android/provider state,
DAVx5, Tasks.org, external network state, the personal Flatpak, credentials,
or the pending credential artifact.

## Reproduced defects and focused fixes

The local method matrix reproduced two protocol defects:

- `HEAD` was advertised as a read-compatible method by clients but returned
  `405`; it now follows the GET representation status and headers while
  suppressing the body, including error bodies.
- GET/HEAD ignored ETag conditions, and DELETE ignored `If-None-Match`.
  `If-Match` now takes precedence on retrieval, matching `If-None-Match`
  returns `304` with the current ETag, failed matches return `412`, and a
  matching DELETE `If-None-Match` returns `412` without archiving.

The existing wildcard fix remains covered: `If-Match: *` updates and deletes
an existing resource, while `If-None-Match: *` prevents duplicate creation or
deletion of an existing representation.

## Verification

Focused commands, all offline with `LD_PRELOAD` unset:

```text
env -u LD_PRELOAD cargo fmt --all
env -u LD_PRELOAD cargo test -p any-cal-dav-server --tests --offline
env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --all-targets --offline -- -D warnings
env -u LD_PRELOAD cargo test -p any-cal-app --test app --offline
```

Results: 32 DAV tests passed, clippy passed with `-D warnings`, and 41 app
integration tests passed. The matrix covers GET/HEAD/PUT/DELETE/REPORT/
PROPFIND, malformed and unsupported methods, ETag matching and stale-write
rejection, wildcard behavior, no-mutation-on-failure, response status/content
type/Allow contracts, and deterministic report/discovery behavior.

## Remaining date-precondition finding

`If-Unmodified-Since` was reviewed as part of this lane but is not implemented
by the current repository contract: `StoredResource` has no persisted
last-modified timestamp, and emitting a synthetic wall-clock value would make
conditional writes unstable across restart and remote Anytype hydration. The
handler therefore does not claim date-precondition support yet. A follow-up
design must add a durable, source-consistent modification timestamp (or
explicitly reject the header) before advertising date precedence. This is a
known research gap, not evidence of live-client interoperability.

No external interoperability claim is made by this receipt.
