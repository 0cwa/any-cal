# DAV authentication-audit configuration precedence — current evidence

Date: 2026-09-20  
Scope: local/offline synthetic configuration only. No external clients or
network, Anytype credentials, Android/provider state, personal Flatpak/device,
or pending credential artifact were accessed.

## Resolved precedence

The service resolves settings in this order:

```text
built-in defaults → config file → environment → command line
```

The authentication-audit switch is default-off. `emit_auth_events=true` may be
set in a file or with `ANY_CAL_EMIT_AUTH_EVENTS=true`; the command-line
overrides are `--emit-auth-events` and `--no-emit-auth-events`. The latter wins
when both are supplied in sequence. Other CLI settings follow the same final
override rule. Runtime credentials remain environment/configuration inputs and
are not added to CLI diagnostics.

## Reproduced defects and focused fixes

Synthetic inputs showed that invalid unknown configuration keys and unknown
command-line options were echoed in `ConfigError`. Since operator-controlled
keys/options can contain pasted credentials or sensitive paths, both errors now
use bounded generic messages. File-read failures likewise no longer echo the
requested path. The effective `check` diagnostic now reports only the boolean
`auth_audit_enabled` state; it does not print credentials or raw configuration
values.

## Evidence

Commands were run with dynamic loader injection unset and a fresh temporary
Cargo target:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-config-target cargo fmt --all -- --check
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-config-target cargo test -p any-cal-app --test app --offline
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-config-target cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
```

Results:

- formatting passed;
- 32 provider-free app integration tests passed;
- clippy passed with warnings denied;
- file → environment → CLI precedence passed for the audit switch and a
  normal service setting;
- unset defaults stayed audit-disabled;
- invalid booleans, unknown keys/options, and missing files failed closed with
  bounded diagnostics that did not contain the synthetic sentinel;
- effective diagnostics exposed only redacted endpoint data, configuration
  booleans, and bounded operational values.

No credential value, credential-bearing path, external state, or live service
was included in the evidence.

## Boundary

This receipt proves local configuration behavior only. It does not claim
production service-manager environment precedence, persistent telemetry, or
external client validation.
