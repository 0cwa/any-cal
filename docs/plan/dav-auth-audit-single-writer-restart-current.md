# DAV auth audit single-writer/restart — current evidence

Date: 2026-09-20

## Scope and boundary

This receipt covers provider-free Rust tests using synthetic, already-redactable
authentication events. No network, external client, Anytype credential, Android
provider, DAVx5/Tasks.org, personal Flatpak, or pending credential artifact was
read or used. No retention/deletion policy was added.

## Reproduced defect and implementation

Before the fix, eight concurrent direct `AuditEventStore` producers were able
to race its read/temporary-file/rename sequence: the temporary reproduction
observed only 21 persisted records out of 128 expected. The reproduction was
removed after capturing the result; it did not expose event values.

`any_cal_observability::AuditEventWriter` now provides the narrow service-owned
single-writer boundary:

- one process-exclusive `.writer.lock`, created with `create_new` and mode
  `0600`;
- a bounded `sync_channel`, with producers applying blocking backpressure;
- one worker owning all `AuditEventStore` appends, exports, and checkpoints;
- monotonic durable sequence numbers and no concurrent record rewrites;
- explicit graceful shutdown that drains queued commands and releases the lock;
- restart recovery that preserves prior records;
- explicit stale-lock recovery only when the recorded process is absent;
- health counters for accepted, persisted, failed, queued, capacity, and last
  sequence, plus an event-count/last-sequence checkpoint;
- worker and store errors fail closed; no automatic stale-lock deletion or
  retention behavior is introduced.

## Focused evidence

Commands were run with dynamic-loader injection unset and an isolated target:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-writer \
  cargo test -p any-cal-observability --offline
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-writer \
  cargo clippy -p any-cal-observability --all-targets --offline -- -D warnings
env -u LD_PRELOAD cargo fmt --all -- --check
```

Results:

- 20 observability unit tests passed;
- clippy passed with warnings denied;
- formatting passed;
- eight concurrent producers persisted exactly 128 synthetic events with
  sequence values 1 through 128, bounded capacity 2, zero failures, zero
  queued events after export, and no redacted value leakage;
- graceful shutdown removed the writer lock, restart preserved the two prior
  records, and checkpoint/export remained ordered;
- an active/current-process lock was refused, while a synthetic absent-process
  lock required explicit stale-lock recovery before restart;
- existing corruption tests continue to reject interior corruption without
  deleting or rewriting the corrupt source.

## Remaining boundary

The writer is a local process-owned primitive. Service-manager integration,
operator retention policy, and persistent telemetry backends remain separate
decisions and are not claimed by this receipt.
