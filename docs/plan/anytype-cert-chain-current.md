# Anytype coordinator certificate-chain and trust-resolution report

**Work unit:** `anytype-certificate-chain-trust-resolution`  
**Run date:** 2026-09-20  
**Scope:** read-only DNS and TLS metadata for the three exact coordinator names
and ports already present in the finite allowlist. No credentials, HTTP
requests, trust-store changes, verifier changes, or plaintext fallback were
used.

## Result

All six listeners present a single self-signed leaf certificate. There is no
CA/intermediate chain to validate against the Linux or Android public trust
stores. The certificate has no X.509 extensions, including no DNS Subject
Alternative Name (SAN), and its subject/issuer contain only a serial-number
attribute. Consequently, an ordinary hostname-and-public-CA HTTPS client is
not a compatible trust model for these listeners.

The three coordinator certificates are distinct. The certificate is identical
between ports 443 and 1443 for each coordinator. The observed certificate
metadata is:

| Endpoint group | Resolved address | Subject/issuer | Validity | Redacted SHA-256 fingerprint |
| --- | --- | --- | --- | --- |
| coordinator1 | `5.39.38.186` | self-signed; serial-number subject ending `…362` | 2026-08-18 20:33:13 UTC – 2126-07-25 21:33:13 UTC | `97:A6:03:AD:…:EF:05:2C` |
| coordinator2 | `51.68.133.54` | self-signed; serial-number subject ending `…576` | 2026-08-18 20:34:10 UTC – 2126-07-25 21:34:10 UTC | `23:92:34:85:…:A3:20:79` |
| coordinator3 | `51.77.75.35` | self-signed; serial-number subject ending `…917` | 2026-08-18 20:34:28 UTC – 2126-07-25 21:34:28 UTC | `C7:89:A0:2D:…:66:F5:ED` |

All six handshakes negotiated TLS 1.3. The certificate public key is ECDSA
P-256. OpenSSL's platform trust check returned `18 (self-signed certificate)`
for each coordinator on port 443; port 1443 presented the same certificate as
its corresponding 443 listener. No HTTP request was sent.

## Exact read-only commands

The following commands were run from the workspace host with no credential
environment or application profile. The certificate PEM bodies were inspected
only as a pipeline into `openssl x509`; no private material was involved and
no certificate file was retained.

```text
getent ahosts prod-any-sync-coordinator1.ovh.toolpad.org
getent ahosts prod-any-sync-coordinator2.ovh.toolpad.org
getent ahosts prod-any-sync-coordinator3.ovh.toolpad.org

openssl s_client \
  -connect prod-any-sync-coordinator1.ovh.toolpad.org:443 \
  -servername prod-any-sync-coordinator1.ovh.toolpad.org \
  -showcerts -verify_return_error \
  -verify_hostname prod-any-sync-coordinator1.ovh.toolpad.org </dev/null

openssl s_client \
  -connect prod-any-sync-coordinatorN.ovh.toolpad.org:PORT \
  -servername prod-any-sync-coordinatorN.ovh.toolpad.org </dev/null 2>/dev/null |
  openssl x509 -noout -subject -issuer -serial -dates -fingerprint -sha256 \
    -ext subjectAltName -ext basicConstraints -ext keyUsage
```

The metadata command was run for `N=1,2,3` and `PORT=443,1443`. The trust
command was run for each coordinator on port 443. Representative trust output
was:

```text
depth=0 serialNumber=<redacted>
verify error:num=18:self-signed certificate
Verification error: self-signed certificate
Verify return code: 18 (self-signed certificate)
```

## Protocol and resolution implications

The evidence does not support installing a public root, copying the presented
certificate into Any-Cal, or putting a normal HTTPS reverse proxy in front of
the coordinator listener. The official Any-Sync documentation describes
coordinator communication as libp2p/Any-Sync transport, and the libp2p TLS
specification intentionally uses a self-signed certificate for peer identity;
peer identity is authenticated from the libp2p public-key extension and the
expected peer ID, not from a Web-PKI hostname chain:

- [Any-Sync netcheck](https://github.com/anyproto/any-sync-tools/blob/main/any-sync-netcheck/README.md)
- [libp2p TLS specification](https://github.com/libp2p/specs/blob/master/tls/tls.md)
- [Any-Sync self-hosted network configuration](https://github.com/anyproto/anytype-cli/blob/main/SELF-HOSTED.md)

The official self-hosted configuration exposes peer IDs and addresses rather
than a CA or hostname-verification setting. The public Docker Compose
configuration likewise documents the coordinator as a protocol service, not
as a conventional HTTPS API gateway:

- [Any-Sync Docker Compose](https://github.com/anyproto/any-sync-dockercompose)

Therefore no supported platform-trusted gateway or CA configuration was found
for these exact production coordinator listeners. Android's system trust
store and `rustls-platform-verifier` should continue to reject them when used
as ordinary HTTPS endpoints. A custom CA would not address the missing SAN and
would not implement libp2p peer-ID authentication.

## Required architecture correction

The failed live HTTPS attempt should be classified as a protocol/endpoint
mismatch, not merely an unknown-CA deployment problem. Any-Cal should not use
the coordinator's libp2p TLS listener as an HTTP Anytype API endpoint. The next
safe validation lane is one of:

1. Run the pinned Anytype CLI/API server in the isolated guest and connect to
   its documented local HTTP API over loopback; or
2. Implement the Any-Sync/libp2p client transport with peer-ID authentication
   and the network configuration's exact peer IDs, if direct coordinator
   access is genuinely required.

The existing Rust HTTP transport should retain strict Web-PKI validation for
ordinary HTTPS URLs. No `danger_accept_invalid_certs`, hostname bypass,
certificate pinning workaround, or plaintext fallback is authorized.

## Cleanup and limits

No VM, trust store, application configuration, certificate file, credential,
or Anytype data was modified. DNS and TLS metadata are point-in-time evidence;
certificate rotation should be rechecked before a future approval-gated live
run. This report does not claim Any-Sync protocol success, Anytype API CRUD,
Android runtime success, or a working trusted gateway.
