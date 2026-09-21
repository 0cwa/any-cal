# Secure Anytype transport for Android

**Status:** native transport and Android verifier initialization are present;
host verifier configuration is tested, while Android trusted-endpoint runtime
acceptance remains pending. No credentials are recorded in this plan.

## Current acceptance gap

`HttpAnytypeTransport` and the Android native bridge accept `http://` and
`https://` endpoints. HTTPS uses rustls with
`rustls-platform-verifier`, validates the endpoint authority and server name,
and applies bounded connect/read/write timeouts. The Android bridge now aligns
with the verifier's JNI 0.22 API, exposes an explicit JVM/context
initialization entrypoint, and resolves the verifier support AAR from the
Cargo dependency metadata. Secure requests fail closed until initialization
has succeeded. Host configuration and parser/error tests pass, but no
Android trusted-endpoint runtime receipt or live Anytype HTTPS run exists;
production HTTPS readiness must not be claimed.

## Evidence-backed options

### Option A — Rust TLS client

Replace the raw socket path with a pinned Rust HTTP/TLS client and build it for
the Android ABIs. `reqwest` documents both Rustls and native-TLS backends,
certificate configuration, timeouts, and proxy environment/configuration:
[reqwest TLS module](https://docs.rs/reqwest/latest/reqwest/tls/),
[reqwest client](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html),
[reqwest overview](https://docs.rs/reqwest/latest/reqwest/).

For Android, the candidate Rustls verifier is
[rustls-platform-verifier](https://github.com/rustls/rustls-platform-verifier),
whose upstream project documents OS-backed verification including Android.
This option keeps the Anytype transport and canonical sync logic in Rust, but
requires an NDK/ABI build, verifier compatibility test, and an explicit proxy
policy. Do not select a crypto provider or verifier version until an API-35
native build is reproducible.

### Option B — Kotlin/Android HTTPS exchange

Keep Rust responsible for request construction, response parsing, canonical
sync, and checkpoints, but implement the `HttpExchange` equivalent in Kotlin
using the Android HTTPS stack. Android's Network Security Configuration
supports custom trust anchors, debug-only overrides, cleartext policy, and
certificate pinning ([official guide](https://developer.android.com/privacy-and-security/security-config)).

This naturally follows Android system trust and proxy behavior, but it widens
the bridge: the Kotlin side must expose bounded request/response bytes and
classified network errors, and request construction must not leak the bearer
token into logs or DTOs. It also creates two HTTP implementations unless the
desktop path is migrated to the same abstraction.

### Option C — Platform-native TLS through a native-TLS crate

Use an Android-native TLS backend and bind the Rust transport to it. This may
follow platform certificate/proxy behavior, but the exact NDK, JNI, and native
library packaging requirements are not present in this workspace. It should
not be selected over Option A or B without an API-35 build receipt.

## Recommendation

Use **Option A for the shared Rust transport**, with Rustls plus a pinned
platform-verifier strategy if the API-35 build proves it can consume the
Android system trust store. Keep **Option B as the fallback** if platform
verification or Android proxy behavior cannot be made reliable in the native
build.

The first implementation should expose a transport-independent Rust trait with
classified errors and a bounded `HttpExchange` seam. It should then have:

1. a desktop Rust TLS implementation;
2. an Android Rustls implementation, or a Kotlin HTTPS exchange selected by a
   compile-time/platform adapter;
3. identical request policy, body limits, redirects, redaction, and error
   classification across both.

Do not silently fall back from HTTPS to HTTP. HTTP may remain an explicit
loopback-only development mode, never a production Android default.

## Required TLS and endpoint policy

### Certificate and hostname validation

- Require `https://` for non-loopback endpoints.
- Use the platform/system trust store by default.
- Verify the certificate chain and requested hostname; never enable an
  “accept invalid certificates” or “accept invalid hostnames” path in release
  code. The reqwest API explicitly warns that disabling hostname verification
  enables man-in-the-middle attacks ([ClientBuilder warning](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)).
- Custom CA roots are an explicit per-endpoint configuration, stored as
  nonsecret certificate material and tested separately. Android's network
  security configuration supports custom trust anchors and debug overrides, but
  that configuration applies to the Android networking stack; a Rust TLS stack
  needs its own verified root integration.
- Certificate pinning is optional and should be an explicit, rotatable policy,
  not an ad-hoc fingerprint bypass. A pin mismatch is a hard failure.
- Reject URL userinfo, malformed authorities, unexpected schemes, and
  cross-origin redirects. The current raw client does not follow redirects;
  preserving that default is safest for the first TLS implementation.

### Proxy behavior

The current `TcpStream` transport has no proxy support. A secure replacement
must choose one explicit policy:

- Android-native exchange: use only the platform-approved proxy configuration
  and report the selected proxy mode without logging credentials.
- Rust client: accept an explicit, bounded proxy configuration from the host;
  do not inherit arbitrary environment variables on Android without a reviewed
  policy.
- Direct mode: disable proxies explicitly and document that local/private
  endpoints may require direct reachability.

Proxy credentials must never be embedded in URLs, DTOs, logs, crash reports, or
request diagnostics. CONNECT/TLS hostname verification still applies to the
origin server; a proxy's certificate must not be treated as the origin's
certificate.

### Timeouts, cancellation, and response bounds

Retain the existing 4 MiB response-body bound and add:

```text
connect deadline
TLS handshake deadline
request/write deadline
response/read deadline
overall operation deadline
cooperative cancellation token
```

Cancellation must stop retries and release the socket; it must not commit a
partial canonical checkpoint. Classify timeout, cancellation, DNS, TLS,
certificate, proxy, HTTP status, malformed response, and body-limit failures
without including tokens or personal payloads.

### Token handling and redaction

- Keep the bearer token outside canonical DTOs and provider projection values.
- Store/retrieve the Android secret through the selected Android credential
  boundary; Android Keystore provides the platform key-management APIs
  ([reference](https://developer.android.com/reference/android/security/keystore/package-summary)),
  but the exact encrypted-secret storage design remains a separate decision.
- Construct the Authorization header as late as possible inside the transport.
- Never include request bytes, Authorization headers, query credentials, or
  token-shaped values in errors, traces, bridge `toString`, or crash reports.
- Use the existing redaction layer for diagnostics, but add transport tests
  that assert the original token and representative bearer values are absent.

## Android packaging constraints

The workspace now has a JNI native bridge and Android native-library
packaging, but a local release build still requires the pinned NDK/Gradle
toolchain. Before claiming Android trust, provision a disposable API-35 build
with pinned:

```text
Rust toolchain + Cargo.lock
Android NDK
Rust Android target triples/ABI set
TLS/verifier/crypto-provider versions
Gradle/AGP/Kotlin versions
```

The build must produce and load the native library for every supported ABI,
and must prove that certificate verification works on the emulator rather than
only on the host.

## Acceptance matrix

### Rust transport tests

- Valid HTTPS certificate and matching hostname succeed.
- Wrong hostname, unknown CA, expired certificate, malformed chain, and pin
  mismatch fail closed.
- Redirect to HTTP, cross-origin redirect, URL userinfo, and unexpected port
  are rejected according to endpoint policy.
- Connect, handshake, read, write, and overall deadlines classify correctly.
- Cancellation releases the operation without committing a checkpoint.
- Body/header bounds reject oversized responses and malformed chunking.
- 401/403/404/409/429/5xx preserve existing error categories.
- Authorization token is absent from every error and diagnostic string.

### Android emulator tests

- API-35 emulator trusts a disposable test CA only when explicitly configured;
  release configuration does not trust debug/user certificates accidentally.
- System trust, custom CA, hostname mismatch, and certificate rotation are
  tested through the selected Rust or Kotlin TLS path.
- Direct, platform proxy, and denied proxy modes are explicit and observable
  without proxy credentials.
- Process death during TLS/request/checkpoint stages resumes safely.
- Native library loads for each supported ABI and reports a bridge/schema
  version mismatch without attempting a network write.

### Anytype safety tests

- No live Anytype credential is used in test fixtures.
- Canonical envelope and sync-store tests pass independently of TLS.
- A transport failure leaves pending operations/checkpoints recoverable and
  does not create provider-side partial state.

## Hard unresolved risks

- Whether the chosen Rustls platform verifier and crypto provider build cleanly
  for the pinned Android ABIs and correctly use the device trust store.
- Whether Android system proxy and user-installed CA behavior must be obtained
  through Kotlin rather than native Rust.
- Secure encrypted token storage and rotation across Android account removal or
  backup/restore.
- Redirect and proxy policy for local Anytype endpoints, IPv6 literals, and
  reverse-proxy deployments.
- Cancellation behavior of the selected HTTP/TLS client during DNS lookup,
  handshake, and blocked socket reads.
- Whether one shared Rust transport or split Rust/Kotlin exchanges has the
  smaller long-term maintenance and test surface.

No live Anytype service or credential was accessed. The next safe action is a
disposable TLS hello-world build and certificate-matrix test, not enabling
production Anytype HTTPS in the current raw transport.
