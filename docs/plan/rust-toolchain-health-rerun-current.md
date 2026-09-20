# Rust toolchain health and recovery rerun

Date: 2026-09-20

## Scope and safety

This run was local and offline. It did not access the Anytype credential
artifact, external services, Android/provider state, DAVx5, Tasks.org, or a
personal Flatpak. No credential values, environment values containing secrets,
or compiler logs were retained.

## Diagnosis

The inherited environment had two preloaded libraries:

- `libhardened_malloc.so`
- `libno_rlimit_as.so`

The default `rustc` resolved to the Homebrew Rust 1.98.1 binary. With the
combined inherited preload setting, `rustc --version` aborted with:

`fatal allocator error: invalid uninitialized allocator usage`

The pinned rustup toolchain was present locally as Rust 1.97.1. The following
secret-free boundary checks passed:

| Check | Result |
| --- | --- |
| pinned `rustc --version`, preloads removed | pass |
| pinned `cargo --version`, preloads removed | pass |
| pinned `rustfmt --version`, preloads removed | pass |
| pinned `rustc --version`, only hardened allocator preload | pass |
| pinned `rustc --version`, only address-limit preload | pass |
| Homebrew `rustc --version`, preloads removed | pass |

This identifies the failure boundary as the combination of the inherited
preloads with the default Homebrew compiler/runtime path. No system-wide
configuration was changed. The healthy execution path uses the already
installed pinned rustup binaries and removes `LD_PRELOAD` for Rust commands.

## Post-edit verification

All commands used the pinned local toolchain, `LD_PRELOAD` unset, and Cargo's
offline mode.

| Verification | Result |
| --- | --- |
| `cargo test -p any-cal-sync --lib --offline` | 16 passed, 0 failed |
| `cargo test -p any-cal-observability --lib --offline` | 12 passed, 0 failed |
| `cargo test -p any-cal-app --test app --offline` | 23 passed, 0 failed |
| `cargo fmt --all -- --check` | pass |
| targeted `cargo clippy` for sync, observability, and app, `-D warnings` | pass |

The two post-edit recovery regressions both passed:

- `recovery_does_not_overwrite_valid_backup_before_republish`
- `unrecoverable_corruption_does_not_leave_a_stale_lock`

## Cleanup and residual boundary

No temporary external or credential state was created. The existing Cargo
build output is ordinary local build state and was not modified destructively.
The host's inherited preload configuration remains unchanged; callers running
Rust commands in this environment must use the recorded pinned-toolchain,
preload-free invocation until the host configuration is separately corrected.
