# Observability service integration

Status: locally verified integration; production operations intentionally
deferred.

The application now owns a bounded `EventBuffer` and `Health` state. Every
handled request receives a generated request ID unless `X-Request-ID` is
provided; `X-Sync-ID` is propagated into correlated events. DAV success and
failure outcomes update health counters, while `/health` and `/status` expose
transport mode, configured space/collections, and bounded counters without
including credentials.

The app exposes explicit hooks for reconciliation reports and uses the
observability crate's atomic backup/restore helpers for recovery artifacts.
Recovery events carry bounded typed error codes and correlation IDs; health
and status expose the last bounded error category plus recovery mode without
tokens or personal fields. Artifact temporary names include process and
timestamp entropy so concurrent backups do not share a staging path.
Durable DAV writes already checkpoint visible state through the sync store;
checkpoint failures become a bounded 500 response and do not publish stale
ETag headers.

Live Anytype metrics, authenticated production telemetry, remote change
streams, and a persistent event sink remain deferred deliberately. Persisting
short-lived request events and health counters would add a second recovery
state whose crash ordering could disagree with the durable sync checkpoint;
the sync checkpoint is therefore the only restart-safe operational state in
this bounded profile. Operators can reconstruct current health after restart,
while the in-process buffer remains bounded and secret-safe.

The app wrapper's backup/restore hooks are executable and emit recovery events,
but no scheduled backup policy, retention/rotation policy, encryption-at-rest,
or restore drill exists. A future observability sink or backup service must
first satisfy the deferred-work matrix in
`docs/plan/observability-recovery.md`; adding a sink without those guarantees
would create an unverified second source of operational truth.
