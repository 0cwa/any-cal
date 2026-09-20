# Service startup/configuration fault boundary — current receipt

Date: 2026-09-20  
Scope: local/offline synthetic fixtures only. No external network, Anytype,
Android, DAVx5/Tasks.org, personal Flatpak/device, or workspace credential
artifact was read or accessed.

## Matrix result

The existing configuration and local service matrix passed, including:

- file → environment → CLI precedence;
- missing/invalid endpoint, API version, Space scope, listen address, and
  collection identifiers;
- loopback, IPv6 loopback, LAN, and reverse-proxy combinations;
- fail-closed authentication and local-health credential separation;
- duplicate authorization headers and request-controlled Space rejection;
- rate-limit and connection-limit configuration;
- structured health/status output and secret-safe `check` output.

Three narrow defects were reproduced and fixed:

1. A malformed file line was copied verbatim into `ConfigError`, which could
   echo operator-supplied credential material. The diagnostic is now a stable
   generic error.
2. `check` printed endpoint userinfo and query/fragment material. Diagnostics
   now retain only scheme, host/port, and path; userinfo and query/fragment
   material are omitted.
3. Explicitly configured empty token, DAV credential, or local-health
   credential values were accepted. Validation now rejects them before startup.

## Verification

Commands (with `LD_PRELOAD` unset):

```text
cargo fmt --all
cargo test -p any-cal-app --test app
cargo clippy -p any-cal-app --all-targets -- -D warnings
```

Result: 27 integration tests passed; clippy passed in a fresh target
directory. The app library's three socket-concurrency tests remain unable to
bind/connect in the restricted runner (`EPERM`); this is an environment
boundary already covered by the host-runtime evidence and is not a
configuration defect.

No credential values, credential-bearing paths, or live service state are
included in this receipt.

## Remaining boundary

This work unit does not claim production deployment, LAN plaintext support, or
external TLS/proxy validation. The existing policy remains fail-closed:
non-loopback binds require explicit LAN permission and a loopback plaintext
backend behind an explicitly configured TLS reverse proxy.
