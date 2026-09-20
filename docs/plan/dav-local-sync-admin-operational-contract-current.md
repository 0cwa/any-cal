# Local DAV sync-admin operational contract

Status: complete for the provider-free, synthetic local-service scope.

Date: 2026-09-20

This document is the operator contract assembled from the health, audit,
export/restore, resilience, and service-wiring receipts listed below. It is
deliberately narrower than a production deployment claim: it does not claim
live Anytype, external DAV-client, Android, DAVx5, Tasks.org, retention, or
multi-user validation.

## Operator surface

The service has two separate authorization planes:

* `/health`, `/status`, and `/ready` are read-only health endpoints. When a
  runtime local-health credential is configured, that credential authorizes
  these endpoints only. The operator credential remains an accepted alternate
  for health, but a health-only credential cannot authorize DAV or sync-admin
  operations.
* `/admin/sync/capabilities` (and the compatibility route `/admin/sync`),
  `/admin/sync/audit`, `/admin/sync/export`, and
  `/admin/sync/restore` require the operator credential. They never accept a
  health-only credential.

The sync-admin routes are intentionally fixed-path operations. The export and
restore artifact is adjacent to the configured checkpoint; requests cannot
select arbitrary filesystem paths. Capability discovery reports only enabled
state, fixed format/version, and bounded counts/metadata.

## Configuration and exposure

Configuration precedence is defaults, then the key/value file selected by
`--config`, then environment variables, then command-line flags. Secrets are
runtime-only values: the Anytype token, DAV/operator credential, and local
health credential are not included in `check`, health, admin receipts, audit
readback, or service events.

Loopback is the default listener boundary. A non-loopback listener is refused
unless the reverse-proxy mode is enabled; proxy mode requires a loopback
Any-Cal listener and a separately configured HTTPS reverse proxy. The local
health credential is distinct from the Anytype token and is not persisted by
the GUI. Direct plaintext LAN exposure is fail-closed.

## Health and readiness

Health responses are uncached, typed JSON and include only bounded state:
service status/readiness, configured capability booleans, transport class,
event/failure counters, sync checkpoint/export readiness, audit state, and a
stable error category when present. Safe request correlation is returned as a
bounded `X-Request-ID`; unsafe, oversized, whitespace, or control-containing
values are not echoed.

Required audit/checkpoint failures make readiness fail closed and prevent
successful DAV mutation from being reported when durable state was not
checkpointed. Optional audit degradation remains observable without falsely
claiming that the whole service is unavailable. Recovery is reflected by a
bounded recovering/healthy state after the durable store is repaired.

## Audit and diagnostics

`GET /admin/sync/audit` accepts only bounded `limit` (1..64) and `after`
sequence parameters. It returns sequence/timestamp, allow-listed operation
and status tokens, bounded category, and a truncated correlation digest. It
does not return raw messages, paths, resource identifiers, collection names,
credentials, or checkpoint contents. Corrupt, duplicate, gapped, unavailable,
or worker-failed audit state returns a bounded unavailable response.

Admin errors use generic bounded status/category responses, `no-store`, and a
safe correlation header. Restore failures do not echo parser errors, paths,
or artifact bytes. Diagnostics are therefore actionable at the category and
recovery-step level without becoming a secret or PII transport.

## Export, restore, and recovery procedure

1. Query the authenticated capabilities route and confirm export/restore are
   enabled.
2. POST the empty-body export operation. The service atomically writes the
   versioned `any-cal.sync-export` artifact beside the configured checkpoint.
3. Preserve that artifact using an operator-controlled backup process. The
   service does not schedule, rotate, retain, or delete backups.
4. After a controlled stop or before recovery, POST the empty-body restore
   operation. The service validates format/version, state version, canonical
   payload digest, path safety, and complete state before publication.
5. Restart or reopen the service and query readiness. The active in-memory
   state is replaced only after validated publication, so malformed, tampered,
   truncated, incompatible, symlinked, or failed artifacts leave the active
   state unchanged.

Checkpoint writes are atomic and occur before a successful mutation is
reported. A write, rename, or directory-sync failure yields a bounded recovery
error and no success `ETag`. Startup refuses invalid checkpoint/audit state;
stale locks have an explicit exact-path recovery operation. Normal shutdown
closes the in-process writer, and reopening preserves sequence/checkpoint
state.

## Explicit deferrals

The following policies are intentionally not silently implied by this
contract:

* automatic retention, rotation, scheduling, or deletion;
* encryption at rest, key management, and production backup custody;
* multi-user administrator roles or audit authorization policy;
* live Anytype transport/CRUD and external DAV-client interoperability;
* Android provider, DAVx5, Tasks.org, and device-account validation;
* production load limits beyond the bounded in-process queue and request
  limits exercised by tests.

## Evidence and consistency check

The contract cross-checks these current receipts:

* `dav-local-sync-admin-health-contract-current.md`
* `dav-local-sync-admin-audit-readback-endpoint-current.md`
* `dav-local-sync-admin-audit-quota-health-resilience-current.md`
* `dav-local-sync-admin-audit-backup-restore-current.md`
* `dav-local-sync-service-export-integration-current.md`
* `dav-local-sync-service-wiring-current.md`

Those receipts report passing focused sync, observability, and application
tests, formatting, and applicable Clippy checks with `LD_PRELOAD` unset and
disposable temporary state. A fresh full application rerun in this lane was
not counted as evidence because the shared temporary quota was exhausted
(`os error 122`); it produced no source diagnostic. Existing receipts remain
the authoritative test evidence.

No code defect was reproduced and no source change was required. This lane
used no external network, live Anytype, real credentials, Android/provider
state, DAVx5/Tasks.org, personal Flatpak state, pending credential artifact,
or destructive cleanup. Temporary build output created by this lane was
removed where possible.
