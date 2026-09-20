# Observability correlation/cardinality boundary

Status: complete for the bounded local synthetic scope on 2026-09-20.

This receipt covers only in-process Rust observability behavior. It does not
inspect the pending workspace credential artifact and makes no external,
Anytype, Android, DAVx5/Tasks.org, device, Flatpak, telemetry, or retention
claim.

## Matrix

| Boundary | Synthetic exercise | Result |
| --- | --- | --- |
| Correlation | 8 concurrent workers emitted 32 timeout events each with distinct request, sync, and correlation identifiers | 256 events retained; every event preserved its request-to-sync worker pairing and had a correlation identifier |
| Report cap | 133 synthetic report events | 128 retained; report remains marked bounded |
| Event cap | Existing bounded buffer coverage plus application fault matrix | Existing 512-event retention invariant remains covered |
| Field bounds | Long request, sync, and correlation values; repeated credential-like fields | Identity fields are redacted and capped at `MAX_FIELD` plus truncation marker; messages remain capped at `MAX_MESSAGE` |
| Duplicate/ordering attribution | Concurrent events were checked by their explicit worker correlation, and FIFO/bounded behavior was exercised without cross-request attribution | No duplicate or cross-request attribution was produced by the synthetic matrix; concurrency order is intentionally treated as scheduling-dependent |
| Serialization | Fixed event JSON-shape test and bounded-field tests | Stable field order and bounded diagnostic values |
| Secret safety | Repeated `token`, `api_key`, authorization, and PII-shaped synthetic values | Values are replaced before diagnostics are bounded or serialized |

## Reproduced defect and focused fix

The redaction loop previously processed only one occurrence of a sensitive
field. Extending it to repeated fields caused a second defect: after replacing
`token=...`, the replacement still contained the field name, so the loop could
reprocess the replacement forever. The implementation now advances a byte-safe
cursor and skips an already-redacted value. Identity labels passed through the
event constructors are also redacted and bounded by `MAX_FIELD` (128 Unicode
characters, plus `…` when truncated).

Focused evidence:

```text
env -u LD_PRELOAD cargo test -p any-cal-observability --lib -- --test-threads=1
16 passed, 0 failed, 0 ignored
env -u LD_PRELOAD cargo clippy -p any-cal-observability --lib -- -D warnings
passed
env -u LD_PRELOAD cargo test -p any-cal-app --test app -- --test-threads=1
28 passed, 0 failed, 0 ignored
```

The implementation change is limited to `crates/observability/src/lib.rs` and
does not introduce a persistent sink, retention policy, or production
telemetry claim.
