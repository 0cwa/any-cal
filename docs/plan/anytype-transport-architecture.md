# Anytype transport architecture (pinned CLI and production boundary)

Updated 2026-09-20. This document records the supported transport boundary
for Any-Cal. It is deliberately separate from the Any-Sync coordinator
transport: the coordinator listeners are libp2p/peer endpoints and are not
ordinary HTTP API servers.

## Decision

Any-Cal should consume Anytype through the embedded headless Anytype CLI/API
server, or through a supported HTTPS HTTP gateway that exposes the same API.
It must not send HTTP requests to the production Any-Sync coordinator names,
copy their self-signed certificates into the trust store, disable hostname or
chain verification, or treat the coordinator port as a REST endpoint.

The desktop path is therefore:

```text
Any-Cal (Rust adapter) --loopback HTTP or verified HTTPS--> anytype serve
                                                   |
                                      Any-Sync/libp2p peer network
```

The Android path should use the same Rust adapter boundary behind the Kotlin
bridge. It should connect to a user-approved, verified HTTPS gateway or to a
reachable headless CLI instance; it should not implement Any-Sync/libp2p in
the Android companion as part of this project. The API token crosses the
bridge only as an in-memory request credential and is never exposed to the
provider/UI adapters or diagnostics.

## Evidence for the pinned local service

The repository's pinned binary is `.local/anytype-test-v2/cli/anytype`, a
stripped static `x86-64` executable reporting `anytype-cli v0.3.6
(2026-06-17)`. The following commands are safe and do not require a login or
write data:

```text
.local/anytype-test-v2/cli/anytype --version
.local/anytype-test-v2/cli/anytype serve --help
.local/anytype-test-v2/cli/anytype auth --help
.local/anytype-test-v2/cli/anytype auth apikey --help
```

The pinned CLI advertises these listeners:

| Listener | Purpose | Any-Cal policy |
| --- | --- | --- |
| `127.0.0.1:31010` | gRPC server used internally by the embedded service | Do not call directly |
| `127.0.0.1:31011` | gRPC-Web server | Do not call directly |
| `127.0.0.1:31012` | HTTP API | Supported adapter target |

`anytype serve --listen-address host:port` changes the HTTP API listener.
The service still initializes its internal gRPC listener(s), so a startup
health check must account for all required listeners rather than only probing
the selected HTTP port. A disposable startup attempt from this restricted
workspace reached the CLI and failed at `127.0.0.1:31010` with
`Operation not permitted` while creating a socket; this is host execution
policy evidence, not an Anytype protocol failure.

The upstream CLI documents the same three-port layout, loopback defaults, the
HTTP API on 31012, and the `--listen-address` override in its README:
[anyproto/anytype-cli](https://github.com/anyproto/anytype-cli#network-configuration).

The official API documentation currently uses port 31009 in examples because
that is the desktop API default. That is not the headless CLI port. The
endpoint must be configurable and the selected runtime mode must be explicit;
never silently combine desktop `31009` with the headless CLI's gRPC service.
See [Anytype API object operations](https://developers.anytype.io/docs/guides/get-started/objects/)
and [Anytype API authentication](https://developers.anytype.io/docs/guides/get-started/authentication/).

## HTTP API contract used by the adapter

The pinned v1 contract is:

```text
GET/POST /v1/spaces/{space_id}/objects
GET/PATCH/DELETE /v1/spaces/{space_id}/objects/{object_id}
Anytype-Version: 2025-11-08
Authorization: Bearer <API key>
```

The adapter encodes space and object IDs as path segments, sends bounded JSON
requests with `Content-Length`, and accepts only bounded, well-framed
responses. It supports Content-Length, chunked, and connection-close response
framing; rejects conflicting or incomplete framing; enforces a body limit and
socket read/write deadlines; and maps 401/403/404/409/429/408/5xx to typed
transport errors. Token material is not included in errors, receipts, health
output, or event logs.

The API's documented delete operation archives the object. A successful
delete may return the object or `204 No Content`; the adapter creates a
minimal archived tombstone for the latter. Because the API does not provide a
usable ETag/revision precondition in the observed run, sync must retain its
later-write-wins policy and verify ambiguous mutations by bounded reads.

The request exchange is intentionally synchronous and connection-scoped. The
adapter opens a bounded TCP/TLS connection per request rather than assuming
that a persistent HTTP/1.1 connection can be safely reused across all server
versions. This is slower than pooling but makes framing and retry boundaries
explicit; pooling is a separate optimization after live interoperability
evidence.

## Authentication and token lifecycle

The headless service uses a dedicated bot account, not the user's desktop
identity. The lifecycle is:

1. Start the isolated headless CLI.
2. Create or log in a bot account and join only the selected Space.
3. Create an API key through `auth apikey create`.
4. Supply the key to Any-Cal out-of-band (environment, OS secret store, or
   an Android Keystore-backed encrypted configuration).
5. Send it only as a Bearer header to the configured Anytype API origin.
6. Revoke it with `auth apikey revoke` during teardown or rotation.

The CLI documents account-key storage in the system keyring with a config
fallback. Any-Cal must not scrape CLI state, read a user's desktop keyring,
or persist API keys in its ordinary configuration file. The desktop UI may
accept a token transiently only to hand it to a secret-store/service layer;
the Rust core owns the authenticated transport. Android uses Keystore-backed
storage and a short-lived native handle; the Kotlin provider code receives
only projection data.

## Health and startup contract

Health is a three-level check, not merely “the TCP port opened”:

| Level | Check | Meaning |
| --- | --- | --- |
| process | CLI process alive and expected state root | service launched |
| transport | TCP connect to the configured HTTP origin and a bounded request | API listener is reachable and framed |
| authenticated | `GET /v1/spaces` with the configured key, no mutation | key is valid and the account can access the service |

The authenticated check must be read-only, redacted, and rate-limited. A
successful process/transport check must not be reported as Anytype-ready.
Startup should fail closed if the configured origin is not loopback for the
local mode, if HTTPS verification cannot be established for remote mode, if
the API version is unsupported, or if the account/Space cannot be read.

The CLI service's gRPC listener means a health harness running in a restricted
container or sandbox may fail before HTTP is available. The harness should
report the exact failed listener and preserve the distinction between host
socket permission failure, process failure, TCP refusal, TLS verification,
HTTP authentication, and API schema failure.

## Desktop and Android deployment modes

### Desktop

Preferred deployment is a separately managed headless CLI process on the same
machine, bound to loopback, with Any-Cal also bound to loopback. A reverse
proxy may provide LAN HTTPS for DAV clients, but it must terminate TLS and
forward to Any-Cal; Any-Cal must not expose a plaintext non-loopback listener.
The headless CLI's API key is kept outside normal Any-Cal config and injected
only into the service runtime.

### Android

Android should not run the CLI as a child process. The native companion should
use the Rust HTTP adapter over verified HTTPS to a user-managed headless
gateway, or use an explicitly configured local network endpoint with the same
certificate and authentication requirements. The Kotlin bridge transports
typed request/response envelopes and capability/health results; it does not
transport raw HTTP framing, API keys to provider adapters, or Any-Sync peer
credentials.

If the user wants fully offline Android operation, that is a separate product
decision requiring an embedded Any-Sync-compatible runtime or a supported
Anytype mobile API. The current project does not claim that capability.

## What remains unproven

- The live production coordinator certificates are self-signed peer
  certificates without normal Web-PKI identity. They are not accepted as an
  HTTP API trust chain.
- The pinned CLI's live local HTTP API was exercised in the isolated probe
  environment, but this restricted workspace cannot bind its required gRPC
  socket; a host/emulator run is still required for startup/health evidence.
- Remote HTTPS gateway certificate provenance, Android trust-store behavior,
  proxy authentication, and reconnect behavior need a separate bounded test.
- Live rate-limit thresholds, concurrent mutation semantics, change streams,
  ETag/revision support, and richer property writes remain separate probes.
- Desktop port 31009 and headless port 31012 must not be treated as
  interchangeable until a runtime capability check proves the selected mode.

## Required implementation gates

1. Keep `HttpAnytypeTransport` behind the injected `AnytypeTransport` trait;
   no coordinator-specific fallback.
2. Add a transport health result that distinguishes process, TCP/TLS, HTTP,
   auth, and schema failures without revealing credentials.
3. Add a host/emulator startup test for the pinned CLI that verifies 31010,
   31011, and 31012 behavior and then performs only authenticated reads.
4. Add a verified-HTTPS gateway test for desktop and Android before allowing
   non-loopback configuration.
5. Keep live mutation tests approval-gated and disposable; this document and
   the read-only health gate must never create or modify Anytype objects.
