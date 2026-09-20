# Anytype live-validation current state

Updated 2026-09-20. This is an evidence ledger for the Anytype/live-transport
lane. It does not promote fake-transport or localhost evidence into a live
Anytype claim.

## Result

The bounded Anytype CRUD/archive lane is **complete** in a fresh disposable
microVM. A fresh bot account and API key were created inside the VM, two marked
`page` objects were created/read/PATCHed/read-after-update/archived, and the
normal relist returned zero objects. The key, account state, logs, service, and
microVM were removed after verification. The broader live lane remains
incomplete because concurrency, retry/rate-limit, change-stream, ETag/revision,
and HTTPS/TLS behavior were not exercised.

No personal Flatpak, personal Anytype Space, Android account, or DAV account
was used.

## Evidence matrix

| Area | Current evidence | Status | What remains unproven |
|---|---|---:|---|
| API discovery | The v2 disposable runtime logs contain successful, framed `GET` responses for spaces, one Space, types, objects, and properties; the report was redacted and hashed. | Partial | Independent replay of the report, live response schema beyond the captured run, and current endpoint availability. |
| Schema hydration | Fresh runtime observed 11 types and 30 properties, including `description` text and `done` checkbox. | Partial | Contact/Task schema suitability beyond the bounded page projection, unknown-property round-trip behavior. |
| HTTP framing | 15 adapter transport tests pass, including Content-Length, chunked, close-delimited, pagination, bounded bodies, malformed responses, and typed statuses. | Proven locally | Live HTTPS/TLS and server-specific framing behavior. |
| HTTPS/TLS | `HttpAnytypeTransport` now accepts HTTPS through rustls/platform verification with hostname validation and bounded socket timeouts; the certificate/hostname integration test is present but ignored in the restricted socket environment. | Implementation present; acceptance gap | Host certificate matrix, Android trust-store/runtime validation, proxy policy, and live Anytype HTTPS behavior. |
| CRUD/archive | Fresh disposable live run: two creates (201), reads (200), two PATCHes (200), two archives (200), and final relist total 0. | Proven for bounded page payload | Hard delete, delayed visibility, and richer schemas remain unproven. |
| Retry/rate limits | Fresh disposable live read batch returned five 200 responses with no `Retry-After` or rate-limit headers; scripted 429 classification and bounded retry/reconciliation tests pass locally. | Partial | Live thresholds, write-specific quota behavior, and server backoff policy remain unproven. |
| Conflict semantics | Fake stale-drift and no-implicit-overwrite tests pass; live responses exposed no ETag/revision fields. | Local only | Live concurrent PATCH/delete behavior and server-side conflict semantics. |
| Checkpoint/restart | 12 sync tests cover pending operations, atomic publication, backup recovery, corruption, tombstones, retries, and restart. | Proven locally | Reopen/restart against a live Anytype API and server-side visibility semantics. |
| Backup/recovery | Observability tests cover atomic backup/restore, symlink rejection, and concurrent staging paths. | Proven locally | Operational backup scheduling, retention, restore drill, and live-state consistency. |
| Service integration | Fake Anytype repository is wired through the DAV service with diagnostics and checkpoint hooks. | Proven locally | Live Anytype service startup/health and LAN deployment with TLS reverse proxy. |

## Runtime and authority findings

The completed bounded run is recorded in
`docs/plan/safe-change-run-anytype-crud-20260920.json`. The earlier approved
artifact remains historical and must not be reused because it references the
abandoned credential/runtime.
Its CRUD batch allows exactly two uniquely marked `page` objects, one
Contact-like and one Task-like, using only the previously extracted
`description` text and `done` checkbox properties. It requires before/after
counts, marked-ID allowlisting, archive/relist evidence, and cleanup receipts.
It forbids schema creation, unrelated objects, personal data, Android writes,
and raw credentials.

The fresh run used a finite egress policy: DNS plus
TCP 443/1443 to the three exact Anytype coordinator names, default deny, and
no ingress. The fresh sparse sandbox booted with those rules and no prior
credential mount. The CLI service was started inside the VM before account
provisioning; all bounded API calls then completed.

The live archive response returned the pre-archive object body (HTTP 200); a
subsequent GET exposed `archived=true` and normal relist returned zero objects.
No ETag header or revision field was observed. The previously used credential
artifact remains prohibited and was not read, copied, or used.

## Local validation run (2026-09-20)

These package tests passed offline:

```text
any-cal-anytype-adapter: 12 adapter + 15 HTTP transport tests
any-cal-sync:           12 tests
any-cal-observability:  12 tests
```

The app package test command was also attempted. Three listener tests failed
before exercising application behavior because the restricted host denied
socket setup with `Operation not permitted`; five framing tests passed. This
is an execution-environment limitation, not evidence of a live Anytype
failure, and should be rerun in the approved host/emulator test context.

## Required next actions

1. Preserve the live result as evidence; do not reuse the removed key/account.
2. Preserve `docs/plan/safe-change-run-anytype-rate-limit.json` as the bounded
   live rate-limit receipt. Do not infer a threshold or write-specific quota
   policy from the absence of 429 in five normal reads.
3. Decide whether the adapter should model `DELETE` as an archived object
   followed by a verification read, since the live delete response was not a
   tombstone and no ETag/revision was provided.
4. Complete the separate `anytype-https-transport` host certificate matrix and Android trust/runtime gate before any production endpoint claim.

## Source records

- `docs/plan/anytype-probe.md`
- `docs/plan/safe-change-run-anytype-live.json`
- `docs/plan/safe-change-run-anytype-crud-20260920.json`
- `docs/plan/safe-change-run-anytype-rate-limit.json`
- `docs/plan/sync-durability.md`
- `docs/plan/observability-recovery.md`
- `.local/anytype-test-v2/metadata.txt` (metadata only; never expose the
  credential file)
