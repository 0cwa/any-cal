# Local backup and retention-integrity validation

Status: local integrity validation passed; retention/rotation remains an
explicitly deferred production capability.

This receipt covers only disposable synthetic fixtures under the system
temporary directory. It did not inspect or modify workspace credential
artifacts, external services, Anytype state, Android state, DAV clients, or
the user's Flatpak.

## Evidence

| Check | Result | Evidence |
| --- | --- | --- |
| Atomic backup publication and restore | pass | `any-cal-observability` atomic backup/restore test; sibling temporary file, file `fsync`, atomic rename, and parent-directory `fsync` are exercised. |
| Concurrent publication | pass | Eight synthetic backup operations use distinct temporary names and preserve each payload. |
| Checkpoint corruption recovery | pass | `any-cal-sync` tests recover a valid `.bak` after primary truncation and reject corruption in both copies without leaving a stale lock. |
| Symlink refusal | pass | Leaf source/destination symlinks and a symlinked parent directory are refused. The parent-path case was fixed in this validation. |
| Failure reporting | pass | App recovery wrappers emit fixed, bounded `recovery.backup`/`recovery.restore` events; observability and app suites verify redaction and typed recovery classification. |
| Corruption detection of generic artifacts | bounded | The generic observability helper copies bytes and cannot infer whether an arbitrary configuration/checkpoint format is semantically valid. Format-level validation remains the responsibility of the consumer; `SyncStore` performs that validation for JSON checkpoints. |
| Retention/rotation and stale artifacts | deferred | No production retention or deletion API exists. Temporary artifacts are uniquely named and do not affect publication, but stale temporary files are not automatically aged or removed. No user or credential state was deleted. |

## Verification commands

All Rust commands used the pinned toolchain with `LD_PRELOAD` unset:

```text
cargo fmt --all -- --check
cargo test -p any-cal-observability --offline       # 13 passed
cargo test -p any-cal-app --test app --offline      # 23 passed
cargo test -p any-cal-sync --offline                # 16 passed
cargo clippy -p any-cal-observability -p any-cal-app -p any-cal-sync \
  --all-targets --offline -- -D warnings
```

## Boundary

This does not claim scheduled backups, encryption at rest, durable telemetry,
restore drills, or a retention policy. Before enabling rotation, the project
needs an operator-visible age/size/count policy, an atomic manifest or other
integrity record, and a proof that rotation cannot remove the only validated
recovery artifact. The policy must also define how stale temporary files are
identified without following symlinks or touching unrelated files.
