# DAV/HTTP error-mapping and framing boundary receipt

Date: 2026-09-20

## Scope and safety

This was a bounded, local/offline probe. It used only the DAV server and app
test targets with `LD_PRELOAD` unset. It did not read or inspect the pending
credential artifact, use external clients or services, access Anytype,
Android, Tasks.org/DAVx5, the personal Flatpak, or make network calls.

## Reproduced defects and focused fixes

1. The stream reader accepted `Transfer-Encoding: chunked` while implementing
   only `Content-Length` framing. It could therefore treat the request as an
   empty request and leave chunk bytes to poison the next frame. The reader now
   rejects any transfer-encoding header as unsupported and closes the request
   with the existing bounded malformed-request path (`400` at the listener).
2. The HTTP parser accepted invalid protocol versions, control characters in
   request targets/header values, and non-token header names. It now accepts
   only HTTP/1.0 or HTTP/1.1, rejects control/whitespace in targets and values,
   and validates token syntax for methods and header names.
3. Existing timeout mapping had already changed from `500` to truthful `408`.
   The sanitized Tasks.org replay fixture and backend-read matrix assertions
   were stale; they now assert `408`.

## Bounded matrix

The focused tests cover:

- duplicate and truncated `Content-Length` framing;
- unsupported chunked transfer encoding;
- invalid request-line version and control characters;
- invalid header-token/value inputs;
- sequential keep-alive frame boundaries and explicit close semantics;
- status reason phrases for auth, timeout, rate-limit, and unavailable;
- malformed DAV reports/property requests and per-resource `404` responses;
- conditional writes, media-type rejection, XML/multistatus content types,
  and timeout preservation through repository errors;
- deterministic sanitized Android-shaped profile replay.

## Verification

Passed with `env -u LD_PRELOAD`:

```text
cargo fmt --check
cargo test -p any-cal-dav-server --offline --all-targets
  7 unit + 2 android-profile + 4 HTTP + 18 matrix + 3 replay tests
cargo clippy -p any-cal-dav-server --all-targets --offline -- -D warnings
```

The app package's three listener tests still cannot bind sockets in this
restricted sandbox (`Operation not permitted`): malformed-request recovery,
connection overflow, and slow-client concurrency. Five non-socket app tests
pass. Those tests require the separately authorized host-socket lane and are
not claimed here.

## Result

The local DAV parser/error boundary is strengthened and the bounded local
acceptance gates pass. External-client interoperability, host listener
acceptance, and production TLS/auth deployment remain separate gates.
