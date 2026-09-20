# Host socket and TLS integration receipt

**Run date:** 2026-09-20  
**Work unit:** `host-socket-and-tls-integration`  
**Environment:** disposable `ubuntu:24.04` microsandbox VM, 2 GiB maximum memory, 2 vCPUs, no network, repository `target/` mounted read-only. The repository was not mounted writable and no personal Flatpak or Anytype state was used.

## Scope and isolation

The existing test binaries were compiled once on the host with the workspace's
offline dependency cache. Only the resulting `target/` directory was mounted
read-only into each disposable VM. The VM had no external network access; all
socket traffic was loopback-only and all credentials were synthetic test
strings. No Anytype endpoint was contacted.

Host build command:

```text
env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin cargo test --workspace --offline --no-run
env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin cargo build -p any-cal-app --offline
```

Representative VM command shape (the test-specific binary names are omitted
here because they are build artifacts):

```text
msb run --no-net --max-duration 5m -m 2G -c 2 \
  -v /var/home/x/Dev/any-cal/target:/target:ro \
  -w /target/debug/deps ubuntu:24.04 -- <test-binary> --nocapture
```

## Results

### Focused Rust socket/framing suites

| Suite | Result | Evidence |
|---|---:|---|
| Anytype HTTP transport | 15 passed, 1 ignored | Framing, chunked/close-delimited responses, limits, endpoint validation, typed errors, pagination, and token-safe errors passed. |
| DAV HTTP listener | 4 passed | OPTIONS/discovery, 302 `Found` framing, proxy-aware locations, and malformed request handling passed. |
| App listener/policy unit suite | 8 passed | Keep-alive parsing, duplicate/extra-body rejection, 400 recovery, 408 timeout behavior, 503 connection overflow, and comma-separated `Connection` tokens passed. |
| Previously ignored local TLS test | 1 passed | A synthetic self-signed certificate with an incorrect hostname and untrusted CA was rejected as `Unavailable`; the synthetic bearer token was not included in the error path. |

The previously ignored TLS test was explicitly run with:

```text
http_transport-<build-id> --ignored --nocapture
```

### Real app listener process

The actual `any-cal-app serve` binary was launched in the VM in fake transport
mode. The process was exercised over loopback with synthetic runtime-only
credentials.

Observed responses:

- Health without authorization: `401 Unauthorized` with the expected
  `WWW-Authenticate` challenge.
- Health with the synthetic local credential: `200 OK`, bounded
  `Content-Length`, `Connection: close`, and structured JSON health data.
- Two framed requests on one keep-alive connection: two complete `200 OK`
  responses, with the second response closing the connection.
- Malformed request: `400 Bad Request`; the daemon remained able to serve
  subsequent connections.
- Slow partial request: after the configured bounded read deadline,
  `408 Request Timeout` and `Connection: close` were returned.
- With `max_connections=1`, an overlapping second client received `503`
  while the first partial client held the active slot.
- DAV well-known `PROPFIND`: `302 Found`, zero-length body, close framing, and
  the expected local redirect location.
- Captured daemon output contained no synthetic credential values.

### LAN and reverse-proxy policy

The real binary was also checked with synthetic configuration values:

- Non-loopback bind plus `allow_lan=true` without proxy TLS was rejected with
  `LAN HTTP requires reverse_proxy_tls=true; direct plaintext is refused`.
- Non-loopback bind plus `reverse_proxy_tls=true` was rejected because the
  application listener must remain loopback-only behind the proxy.
- Loopback proxy mode without an auth credential was rejected with
  `Missing("auth_credential")`.
- Loopback proxy mode with a synthetic auth credential passed `check` and
  emitted only safe configuration metadata; no token or credential was
  printed.

## HTTPS certificate coverage

The socket-backed HTTPS test now passes in the disposable VM for the negative
case: the transport rejects a synthetic certificate that is both untrusted and
issued for the wrong hostname, and the error does not contain the bearer
token.

The current test harness does **not** install a disposable root CA into the VM
or expose a custom-root verifier seam. Consequently, this receipt does not
claim a valid-trusted-certificate success case, nor does it independently
separate unknown-CA from wrong-hostname failure. Those remain a follow-up
acceptance item before production HTTPS claims. The implementation continues
to use platform certificate verification and hostname validation; no insecure
accept-all path was enabled.

## Cleanup and environment notes

All VMs created by these commands were ephemeral and shut down when the
commands completed. A final `msb list` showed no running VM created by this
lane. One unrelated pre-existing sandbox (`any-cal-live-semantics-20260920`)
was already running and was not touched. No cleanup command was issued against
that unrelated sandbox.

## Gate disposition

- `socket-matrix`: **passed** for listener, malformed/framed requests,
  timeouts, concurrency cap, and loopback policy.
- `auth-framing`: **passed** for synthetic auth, rate/framing behavior,
  bounded timeouts, and secret-safe diagnostics in this lane.
- `https-certificates`: **partially passed**. Negative trust/hostname and
  token-redaction behavior passed; trusted-certificate success and
  independently isolated CA/hostname cases remain open.

No product source changes were made by this lane.
