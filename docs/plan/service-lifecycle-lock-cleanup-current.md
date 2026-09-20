# Service lifecycle, lock, and cleanup receipt

Date: 2026-09-20

Scope: disposable local fake-transport service only. The run used synthetic
Space/checkpoint values in a temporary directory, no network, no Anytype
credentials or data, no Android/provider/client state, no personal Flatpak,
and did not access the pending credential artifact.

## Matrix

| Scenario | Result | Evidence |
| --- | --- | --- |
| Initial start and health | pass | `/health` returned 200; process and checkpoint writer lock were present. |
| Concurrent start with the same checkpoint | pass | Second process exited 2; first process remained healthy and the lock remained owned. |
| Graceful SIGTERM | pass after fix | Process exited 0, loopback health became unavailable, and the checkpoint lock was removed. |
| Restart after graceful stop | pass | A fresh process returned health 200 and recreated the lock. |
| Forced SIGKILL | expected recovery case | Process exited 137, socket became unavailable, and the lock remained as a stale ownership marker. |
| Stale-lock refusal | pass | A new process exited 2 while the stale lock remained; it did not steal another writer's checkpoint. |
| Explicit disposable stale-lock recovery | pass | Removing only the synthetic stale lock allowed a fresh process to start and serve health 200. |
| Final stop and artifact cleanup | pass | SIGTERM exited 0; temporary logs/lock/checkpoint directory were removed and the root directory was absent. |

## Fix

The original process used a blocking listener and default signal termination.
SIGTERM therefore bypassed normal Rust drops and left the checkpoint writer
lock behind. `crates/app` now installs a Unix SIGINT/SIGTERM handler that only
sets an atomic shutdown flag, polls the listener without blocking, drains
accepted workers, restores the default handlers, and returns through normal
drop paths. SIGKILL remains intentionally unrecoverable by the process and is
handled fail-closed by refusing a second writer until an operator removes the
known disposable stale lock.

## Verification

Passed:

```text
env -u LD_PRELOAD cargo fmt --all -- --check
env -u LD_PRELOAD cargo test -p any-cal-app --offline --test app   # 28 passed
env -u LD_PRELOAD cargo test -p any-cal-sync --offline --lib       # 17 passed
CARGO_TARGET_DIR=/tmp/any-cal-service-target env -u LD_PRELOAD \
  cargo clippy -p any-cal-app --all-targets --offline -- -D warnings
```

The process matrix ran inside a disposable Fedora toolbox container because
the restricted agent sandbox denies loopback socket creation. It used no
network access. The matrix passed with the results above. The workspace's
three direct socket unit tests remain environment-gated in the restricted
sandbox; the same service listener was exercised successfully in the toolbox.

## Operational boundary

Stale locks from SIGKILL, power loss, or equivalent abrupt termination are not
automatically deleted. This is deliberate: an age-only heuristic could steal
a live writer's checkpoint. Recovery must be an authority-gated operation
that first proves the owning process is absent and targets only the exact
checkpoint lock.
