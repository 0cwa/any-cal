# Anytype HTTPS end-to-end validation

Updated 2026-09-20. This record covers one fresh disposable run of the
existing Rust `HttpAnytypeTransport`. It is separate from the earlier local
CLI HTTP CRUD receipt and does not claim Android trust-store compatibility.

The host socket/TLS follow-up in
[`host-socket-current.md`](host-socket-current.md) later passed the listener
matrix and the negative certificate/hostname case. A trusted certificate
success case and Android platform-verifier acceptance remain separate gates.

## Outcome

The run reached all six finite-allowlisted coordinator listeners at TCP level,
but the Rust TLS handshake failed closed before any HTTP request was sent:

```text
prod-any-sync-coordinator1.ovh.toolpad.org:443  TCP reachable
prod-any-sync-coordinator1.ovh.toolpad.org:1443 TCP reachable
prod-any-sync-coordinator2.ovh.toolpad.org:443  TCP reachable
prod-any-sync-coordinator2.ovh.toolpad.org:1443 TCP reachable
prod-any-sync-coordinator3.ovh.toolpad.org:443  TCP reachable
prod-any-sync-coordinator3.ovh.toolpad.org:1443 TCP reachable

Rust HttpAnytypeTransport against coordinator1:443
  list_error category=unavailable
  create_error category=unavailable
  token_redaction_harness=pass

Direct rustls probe against coordinator1:443
  tls_valid_result=handshake:InvalidCertificate(UnknownIssuer)
  tls_wrong_hostname_result=handshake:UnexpectedEof
```

The result is a live transport blocker, not a successful Anytype API probe.
The coordinator certificate chain is not trusted by the Fedora guest's
platform verifier. TLS verification was not disabled, a custom root was not
injected, and the Rust transport did not downgrade to plaintext. Because the
handshake failed first, no live HTTP status, authentication response, CRUD,
read-after-write, retry, or reconciliation behavior was exercised in this
run.

## Isolation and authority

- Runtime: fresh sparse `fedora:42` microsandbox, API-key and account state
  guest-only, no host profile or Flatpak mounted.
- Network: default deny; DNS plus only the three exact coordinator names on
  TCP 443 and 1443; no ingress and no other destination was allowed.
- Binary: pinned Anytype CLI v0.3.6 mounted read-only. The Rust harness was
  built offline from the workspace adapter crate and copied into the guest;
  it received the API key through guest stdin only.
- Scope: one newly created disposable bot account and one API key. No Space
  object was changed because the TLS handshake preceded the HTTP request.
- Secret handling: key/account values, full IDs, raw payloads, and service
  logs were not copied to the host or persisted in this report. Harness output
  contained no credential-shaped value.

## Provisioning and cleanup receipts

The disposable account was created inside the fresh guest and its API key was
generated through the authenticated CLI session. The key was revoked before
teardown; the post-revocation CLI listing reported `No API keys found.` Guest
state, key files, service logs, and the Rust harness were removed. The VM was
stopped and removed. The disposable bot account itself has no active key and
was not connected to personal data; the pinned CLI exposes no account-delete
operation, so account deletion is not claimed.

## Existing local evidence still valid

- Adapter framing, typed auth/forbidden/rate-limit/unavailable errors, timeout
  handling, endpoint validation, and token-safe diagnostics remain covered by
  `crates/anytype-adapter/tests/http_transport.rs`.
- The earlier disposable local-CLI HTTP CRUD/archive receipt remains at
  `safe-change-run-anytype-crud-20260920.json`; it must not be interpreted as
  HTTPS evidence.
- Local trusted-certificate and wrong-host tests remain recorded in
  `trusted-tls-current.md`; they do not overcome the live coordinator's
  unknown issuer.

## Required next action

Obtain the documented Anytype production trust configuration or a supported
HTTPS API gateway whose certificate chains to the platform trust store. Verify
the root/intermediate provenance and scope it to the exact endpoint before a
new approval-gated run. Do not copy a coordinator certificate into the app,
disable hostname/chain verification, or use plaintext as a workaround.
