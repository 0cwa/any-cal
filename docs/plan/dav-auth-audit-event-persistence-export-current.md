# DAV auth audit event persistence/export — current evidence

Date: 2026-09-20

## Scope and boundary

This receipt covers local Rust tests using synthetic, already-redacted DAV
authentication events. It did not read or modify credential artifacts, use
Anytype credentials, contact a network or external client, use Android/provider
state, use DAVx5/Tasks.org, or use the personal Flatpak. No automatic
retention/deletion policy was added.

## Implemented seam

`any_cal_observability::AuditEventStore` is an explicit opt-in local journal:

- private directory (`0700`) and private event/segment files (`0600`);
- each append sanitizes the public `Event` boundary again and publishes the
  complete active segment through a temporary file, `sync_all`, and rename;
- sequence numbers provide deterministic ordering and no-duplicate export;
- a torn final JSONL record is treated as a recoverable crash tail and is
  discarded on reopen; interior corruption is rejected without rewriting the
  source file;
- `export_jsonl()` emits all active and rolled segments in sequence order;
- `rollover()` is operator-triggered and never deletes records;
- no age, size, count, or background deletion policy is present.

## Focused evidence

Commands were run with dynamic loader injection unset:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-target cargo test -p any-cal-observability --offline
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-target cargo clippy -p any-cal-observability --all-targets --offline -- -D warnings
env -u LD_PRELOAD cargo fmt --all -- --check
```

Results:

- 18 observability unit tests passed;
- clippy passed with warnings denied;
- formatting passed;
- synthetic append/export/rollover preserved four ordered events with no
  duplicate sequence values;
- a deliberately torn final record recovered to the prior complete export;
- deliberately corrupted interior data failed closed and remained unchanged;
- raw values supplied through a manually constructed `Event` were redacted at
  the persistence boundary;
- directory/file permission assertions passed on Unix;
- export remained identical before and after rollover.

## Remaining boundary

The store is a local persistence primitive. Wiring it into a service-wide
retention or telemetry backend remains intentionally unimplemented and needs a
separate operator-approved policy. Multi-process writers are not claimed; the
current contract assumes one service-owned writer.
