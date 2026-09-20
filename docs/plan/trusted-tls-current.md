# Trusted TLS validation receipt

**Date:** 2026-09-20  
**Work unit:** `trusted-tls-success-validation`  
**Result:** PASS for the bounded Linux platform-verifier path; Android and live
Anytype HTTPS remain separate gates.

## What was tested

The existing `HttpAnytypeTransport` in
`crates/anytype-adapter/src/lib.rs` was exercised through its real HTTPS path,
including `rustls-platform-verifier`. A temporary harness linked the existing
crate and returned a framed JSON response from a local TLS server. The harness,
private keys, certificates, and test CA were disposable and were not retained.

The test ran in a fresh Fedora 42 microsandbox with:

```text
--no-net
--root-disk tmpfs:256M
--memory 512M
--cpus 1
read-only mount of the temporary harness fixture
```

The host’s personal Flatpak, live Anytype service, Anytype credentials, and
external network were not used.

## Matrix and evidence

The harness generated the CA and server certificate inside the VM. The server
certificate had `CN=localhost` and `DNS:localhost` SAN. For the positive case,
the CA was installed only in the disposable VM’s trust-anchor directory and
`update-ca-trust extract` was run.

The VM receipt was:

```text
CERTIFICATE_SUBJECT=localhost
CERTIFICATE_SAN=DNS:localhost
TLS_HARNESS_RESULT=success
TLS_HARNESS_RESULT=unknown-ca-failure
TLS_HARNESS_RESULT=wrong-host-failure
TLS_HARNESS_RESULT=downgrade-failure
TLS_MATRIX_RESULT=pass
SCRIPT_EXIT=0
HOST_EXIT=0
```

The cases mean:

| Case | Setup | Expected result | Observed |
| --- | --- | --- | --- |
| Trusted success | Matching hostname; disposable CA installed | Framed HTTPS JSON request succeeds | `success` |
| Unknown CA | CA removed from trust store | TLS fails closed as `Unavailable` | `unknown-ca-failure` |
| Wrong hostname | CA trusted; endpoint `wrong.example` mapped to loopback; cert SAN remains `localhost` | Hostname verification fails as `Unavailable` | `wrong-host-failure` |
| No downgrade | Plain local socket used behind an `https://` endpoint | No plaintext HTTP request; TLS fails closed | `downgrade-failure` |

For the downgrade case, the first bytes received by the plain socket were:

```text
16 03 01 00 e7
```

That is a TLS ClientHello prefix, not the ASCII HTTP request prefix
`47 45 54 20` (`GET `). The harness also searched all disposable test output
for its synthetic bearer marker and found no match.

## Reproduction command

The temporary harness was built offline against the current workspace with:

```bash
env -u LD_PRELOAD CARGO_HOME=/var/home/x/.cargo \
  PATH=/home/x/.cargo/bin:/usr/bin:/bin \
  cargo build --manifest-path /tmp/any-cal-tls-harness/Cargo.toml \
  --offline --release
```

The disposable VM matrix was run with:

```bash
msb run --no-net --no-tty --pull never \
  --root-disk tmpfs:256M --memory 512M --cpus 1 \
  --max-duration 1m --timeout 30s \
  --volume /tmp/any-cal-tls-vm-fixture:/fixture:ro \
  fedora:42 -- /bin/bash -lc \
  'set +e; bash /fixture/run-matrix.sh >/tmp/tls.log 2>&1; s=$?; \
   cat /tmp/tls.log; echo SCRIPT_EXIT=$s; exit $s'
```

The command exited zero. The one-off VM was not retained.

## Cleanup and boundaries

The fixture’s exit trap removed the disposable CA anchor, regenerated the VM
trust bundle, removed the temporary `/etc/hosts` mapping, and deleted the
temporary certificate/key directory. The host-side temporary harness and
fixture directory were then removed. No key or certificate exists in the
repository or outside the disposable test area.

This receipt does **not** prove that Android’s platform verifier consumes the
same trust store. API-level Android verifier/runtime testing remains a separate
blocker, as does live Anytype HTTPS against a real endpoint. It also does not
authorize enabling production HTTP downgrade, accept-all verification, or
custom roots without an explicit endpoint policy.
