# Local HTTP health/status contract regression

Date: 2026-09-20

This validation stayed local and used only synthetic configuration/credentials.
It did not read the pending workspace credential artifact, access external
services, use Anytype/Android/DAVx5/Tasks.org state, or use the personal
Flatpak. `LD_PRELOAD` was unset for all Rust commands.

## Results

| Area | Result | Evidence |
| --- | --- | --- |
| Health/status schema | pass | The authenticated and unauthenticated app integration matrix passed; `/health` and `/status` expose only fixed status/configuration booleans, transport, cache, bounded counters, last error category, and recovery label. Secret and raw identifier sentinels were absent. |
| Authentication | pass | Missing, wrong, duplicate, Bearer, Basic, local-only, and global DAV credentials were exercised. Challenges use `WWW-Authenticate`; malformed/duplicate authorization fails closed; local health credentials do not authorize DAV. |
| Framing | pass in-process | Content length, keep-alive/close, duplicate `Content-Length`, extra body bytes, 400 responses, 302 discovery, and 503 reason phrase behavior passed in the framing tests that do not require socket creation. |
| Lifecycle/degraded/recovery | pass | Checkpoint failure returns 500 without publishing in-memory state; reopen/restart preserves state; recovery backup/restore and fault-category tests passed. The disposable installed-service receipt separately covers startup, shutdown, lock release, restart, upgrade, and rollback. |
| Diagnostics | pass | `check`, health/status, event messages, and transport diagnostics redact synthetic credentials, endpoint identifiers, query material, and checkpoint details. |
| Focused quality gates | pass | `cargo test -p any-cal-app --test app`: 28 passed. With a fresh target directory: `cargo fmt --all -- --check` and `cargo clippy -p any-cal-app --all-targets -- -D warnings` passed. |

## Runner boundary

The restricted runner denies loopback socket creation (`EPERM`). Therefore the
three socket-backed library tests for malformed-connection recovery, slow
partial-client concurrency, and connection limits cannot execute in this
runner. They are not treated as passing here. The same installed binary's
socket-backed health/framing lifecycle was already exercised in the disposable
non-privileged toolbox receipt at
[`local-installed-service-health-current.md`](local-installed-service-health-current.md),
including 401/200 health, bounded JSON, shutdown, restart, and package
upgrade/rollback behavior.

No implementation defect was reproduced, so no source change was made.

## Reproduction commands

```text
env -u LD_PRELOAD cargo test -p any-cal-app --test app
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-health-contract-target \
  cargo fmt --all -- --check
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-health-contract-target \
  cargo clippy -p any-cal-app --all-targets -- -D warnings
```

The repository target directory had stale artifacts from a different Rust
compiler; the isolated target directory avoided reusing those artifacts.
