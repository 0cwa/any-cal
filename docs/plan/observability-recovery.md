# Observability and recovery

This document is the operational boundary for the current local profile. It
records what the implementation and local tests prove; it is not a production
operations guarantee.

## Verified local contract

`any-cal-observability` is an isolated, dependency-light seam for service and
GUI consumers.

| Area | Current guarantee | Evidence |
| --- | --- | --- |
| Event buffer | `EventBuffer` retains at most 512 events and drops the oldest entry when full. | `crates/observability/src/lib.rs`, `report_is_bounded` and app integration tests |
| Reconciliation diagnostics | `ReconciliationReport` retains at most 128 events and is marked bounded. Reports are diagnostics, not sync truth. | `report_is_bounded`; `App::record_reconciliation` |
| Message safety | Messages are capped at 256 characters and redact authorization/bearer, token/API-key/password, JSON secret fields, and common contact PII. | redaction unit tests and app secret-safety tests |
| Correlation | Request IDs, sync IDs, durations, classified errors, and recovery correlation IDs are carried through bounded events. | `correlation_flows_through_lifecycle_event`; app correlation test |
| Health/status | In-process counters track events, successes, failures, latency, last success/failure, and last error category. `/health` and `/status` expose bounded, credential-free status. | `Health` tests and app integration tests |
| Recovery actions | Timeout/auth/malformed/archive failures map to bounded retry, re-authenticate, or reconcile actions. | `local_failure_matrix_has_bounded_recovery` |
| Artifact backup/restore | Backup and restore write a sibling temporary file, `fsync` the file, rename atomically, and `fsync` the parent directory. Same-path and symlink-containing artifact paths are refused. | atomic backup/restore, concurrent backup, leaf/parent symlink refusal tests |
| Checkpoint boundary | Sync checkpoints remain the restart-safe state. Diagnostic events and health counters are intentionally in-memory and may be reconstructed after restart. | `docs/plan/sync-durability.md`; app checkpoint/reopen tests |

The application wrapper emits `recovery.backup` and `recovery.restore` events
with typed recovery errors and fixed correlation IDs. No event or report is a
secret store, audit log, or authoritative sync journal.

## Explicitly deferred operations

The following are not implemented and must not be implied by the local tests:

| Deferred capability | Required before claiming support |
| --- | --- |
| Persistent telemetry/event sink | Define a durable schema, crash ordering relative to checkpoints, access control, redaction tests, bounded disk usage, and migration/recovery behavior. |
| Retention and rotation policy | Define operator-visible limits, age/size policy, deletion semantics, and a test proving rotation cannot remove the only recovery artifact. |
| Encryption at rest | Choose and manage a key source, document loss/recovery behavior, and test encrypted backup creation and restore without secrets in logs. |
| Scheduled backups | Choose a service-owned scheduler, locking and failure notification policy, retention interaction, and a restart/overlap test. |
| Restore drills | Run disposable backup corruption, partial-write, rollback, and restart drills with recorded postconditions. Local atomicity is not a restore drill. |
| Live metrics/telemetry export | Define an opt-in transport, endpoint authentication, bounded queue/backpressure, privacy policy, and offline behavior. |

There is currently no network access, remote telemetry, Anytype write, secret
storage, scheduled backup, log shipping, or production deployment in this
module. Operators should make backups of verified local artifacts using an
external policy only after the deferred requirements are designed.
