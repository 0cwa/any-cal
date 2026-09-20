# Local sync-admin audit readback integrity

Status: complete for the provider-free, synthetic scope of work unit
`dav-local-sync-admin-audit-readback-integrity`.

Date: 2026-09-20

## Evidence

- `env -u LD_PRELOAD cargo test -p any-cal-observability --lib`: 26 passed.
- `env -u LD_PRELOAD cargo test -p any-cal-app --test app audit -- --nocapture`: 8 passed.
- `env -u LD_PRELOAD cargo clippy -p any-cal-observability --all-targets -- -D warnings`: passed.
- `env -u LD_PRELOAD cargo fmt --all -- --check`: passed.

The app tests cover synthetic sync-admin capability/export/restore/denial
events, required and optional audit health, restart/readiness, stale-lock
recovery, and bounded health output. The observability tests cover bounded
redaction, typed event persistence, deterministic JSONL export, correlation,
single-writer ordering, restart, rollover, crash-tail recovery, corruption,
and failure classification.

## Reproduced defect and fix

The reader previously treated every malformed final JSONL line as a torn
append. A complete malformed final record could therefore be silently
discarded. It now recovers only when the JSON parser reports an EOF/incomplete
value; complete malformed records fail closed with `InvalidData`.

Readback also previously sorted sequence numbers without validating them.
Duplicate and gapped sequences now fail closed before export/checkpoint, both
when opening the active segment and when reading all segments.

The stored-record envelope rejects unknown fields so incompatible records do
not get silently accepted. No credential, path, payload, or resource identity
is included in this receipt.

## Scope boundaries

All state was disposable and synthetic. No network, live Anytype service,
Android/provider state, DAVx5/Tasks.org, personal Flatpak, pending credential
artifact, raw audit export, automatic retention policy, or production telemetry
was accessed. Temporary fixtures were removed by the tests.

