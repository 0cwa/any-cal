# Current completion audit

Date: 2026-09-20

This is a fresh read-only audit of the production-hardening and live-validation
objective. I inspected the current source, work-unit receipts, Android
validation receipts, and safe-change artifacts, and ran the current offline
test commands. No credential file was read, no product or external state was
changed, and this report is the only file written by this audit.

## Executive result

The project is not complete. The local DAV service, authentication policy,
secret redaction, durable checkpoint layer, observability helpers, fake service
integration, deterministic Android projections, and sanitized client replays
have substantial evidence. A bounded disposable Anytype API run also proves
object CRUD/archive and several important server semantics. A fresh API-35
host-KVM receipt proves that the JNI library loads and that account-scoped
ContactsContract and CalendarContract provider CRUD can work in an app-UID
probe.

The end-to-end product path is still absent: the Rust live wire DTO acceptance
has not been rerun after its latest fix, the real Anytype HTTPS endpoint tested
is a libp2p-style self-signed coordinator listener rather than a supported HTTP
API, and no live DAVx5 or Tasks.org client run exists. The host-context socket
receipt now closes the three listener-test environment gap. Android production
callbacks, Anytype reconciliation, AccountManager lifecycle, and backup/
restore remain separate gates.

## Requirement audit

| Requirement | Classification | Evidence and exact remaining gap |
|---|---|---|
| DAV protocol errors and HTTP framing | **Proven locally; host acceptance incomplete** | `crates/dav-server/src/lib.rs` and `crates/app/src/lib.rs` implement bounded request bodies, read/write timeouts, malformed-request `400`, timeout `408`, connection limits with `503`, keep-alive framing, and truthful reason phrases. DAV, Android-profile, replay, and app integration tests cover discovery, ETags, content types, conditional writes, and per-href errors. `cargo test --workspace --offline` currently fails before these three listener behaviors run because `TcpListener::bind` returns `Operation not permitted`; the focused pure parser/framing tests pass. A host-context socket receipt is still required. |
| LAN authentication and HTTPS | **Local policy proven; production HTTPS incomplete** | The app fails closed on non-loopback LAN binds unless `allow_lan=true` and `reverse_proxy_tls=true`, requires a credential in proxy mode, authenticates DAV and health/status, rate-limits failures, fixes the configured Space scope, and keeps the backend loopback-only behind a reverse proxy. `crates/app/tests/app.rs` has passing auth, IPv6, limiter, scope, and proxy-boundary tests. The app does not terminate TLS; reverse-proxy operation, certificate management, trusted-proxy enforcement, and live LAN deployment were not exercised. |
| Secret-safe operations | **Local and bounded live runs mostly proven; operational audit incomplete** | Redaction, masked health/status, runtime-only local auth, control-character rejection, token-safe transport diagnostics, atomic artifact handling, and packaging secret checks are covered by `crates/observability`, `crates/app/tests/app.rs`, and `scripts/test-packaging.sh`. The fresh CRUD/semantics and HTTPS receipts record no retained credentials and successful key revocation. An earlier CLI provisioning run exposed a disposable key in transient command output (`docs/plan/anytype-cli-live-current.md`); it was revoked and the guest removed, but this means the strict no-secret-output gate is not globally proven. `credential-revocation-cleanup` remains planned and no independent all-artifact credential scan receipt exists. |
| Isolated Anytype CLI/local transport and live CRUD | **Direct API CRUD proven; Rust adapter acceptance incomplete** | `docs/plan/safe-change-run-anytype-crud-20260920.json` proves a fresh finite-egress disposable run: two marked page objects were created (`201`), read, patched (`200`), read after update, archived (`200`), confirmed `archived=true`, omitted from normal relist, and cleaned up. `safe-change-run-anytype-live-semantics.json` additionally proves archive semantics, ignored stale `If-Match`, later-write-wins behavior, and a committed mutation after a client timeout. However, `anytype-cli-wire-acceptance-current.md` records the latest Rust `HttpAnytypeTransport` live create stopping on a typed-property wire mismatch; the current `wire.rs` has a candidate tagged-value encoder, but no post-fix live adapter CRUD receipt exists. |
| Anytype HTTPS/TLS | **Blocked by endpoint/protocol mismatch** | `anytype-https-e2e-current.md` and `safe-change-run-anytype-live-https.json` prove strict Rust TLS reached all six allowlisted coordinator listeners but failed before HTTP with `UnknownIssuer`; the certificate-chain report shows self-signed certificates with no SAN. Verification was not bypassed and no plaintext fallback occurred. These listeners are Any-Sync/libp2p-style endpoints, not proven HTTP API gateways. A supported local CLI HTTP service or platform-trusted HTTP gateway is required; copying certificates, disabling verification, or treating coordinator TLS as REST is not acceptable. |
| Durable checkpoint/restart semantics | **Proven locally; live integration incomplete** | `crates/sync/src/lib.rs` and its 14 current tests prove atomic checkpoint publication, backup recovery, lock exclusivity, corruption handling, pending operations, tombstones, explicit resurrection, and failure non-publication. `crates/app/tests/app.rs` proves checkpoint/reopen behavior in the fake service. The live Anytype run supplied no server revision, transaction, change-stream, or reopen semantics, and the Android bridge is not yet connected to this store. |
| Conflict, timeout, retry, and reconciliation behavior | **Observed policy and local implementation proven; end-to-end incomplete** | Live semantics prove that Anytype accepts stale writes with later-write-wins, has no observed ETag/revision or `Retry-After`, and can commit a timed-out PATCH. The adapter has per-object locks and bounded read-before-retry reconciliation; local tests cover ambiguous mutations, transient classification, stale drift, and no blind retry. Natural live rate-limit behavior, parallel interleaving, and a live adapter retry receipt remain unproven. |
| Observability, recovery, and backup | **Bounded local implementation proven; production operations incomplete** | The operational closure in [`docs/plan/observability-recovery.md`](observability-recovery.md) and [`docs/plan/observability-service-integration.md`](observability-service-integration.md) records the verified caps (512 events/128 report events), redaction, correlation, health/status, atomic `fsync`/rename backup and restore, symlink refusal, and the durable-checkpoint boundary. Focused validation currently passes `any-cal-observability` (12 tests) and the app integration suite (23 tests). Persistent telemetry, retention/rotation policy, encryption at rest, scheduled backups, restore drills, and live metrics remain explicitly deferred; no production operations claim is made. |
| Headless/service integration | **Fake/localhost profile proven** | The app test suite (23 integration tests) passes for fake DAV wiring, health/status, auth, configuration, checkpoint/reopen, Contact/VTODO lifecycles, failure behavior, and recovery hooks; packaging smoke covers staged install/upgrade/uninstall without deleting user data. This does not prove a live Anytype-backed service, reverse-proxy deployment, or a release service manager unit. |
| Android JNI/native packaging | **JNI runtime and both-ABI packaging proven; release lifecycle incomplete** | `host-runtime-current.md` and `inprocess-smoke-current.md` prove API-35 app-process JNI loading and credential-free bridge calls. `bridge-integration-current.md` proves current arm64-v8a/x86_64 APK entries and build/verifier checks. `release-current.md` contains an older x86_64-only lifecycle receipt, now explicitly marked historical. Android TLS trust, real AccountManager lifecycle, backup/restore, and production reconciliation remain open. |
| Android ContactsContract/CalendarContract provider CRUD | **Direct synthetic provider CRUD proven; product sync not proven** | The fresh host-KVM receipt proves app-UID account-scoped Contacts and Calendar create/update/tombstone/delete/cleanup, including all required Calendar sync-adapter URI parameters. Projection/callback and bidirectional reconciliation logic also has provider-free tests. But `AnyCalSyncAdapterService` passes an empty contact list and `NoOpRustSyncBridge`; `AnyCalCalendarSyncAdapterService` uses `UnavailableCalendarBridgeSource`. No Android run has exercised the real `onPerformSync` path with Anytype data, checkpoint advancement, restart, conflict, or observer ingestion. |
| Android bidirectional provider path | **Provider-free only; blocked live** | `android-bidirectional-provider-e2e` proves deterministic hash/echo/tombstone/generation decisions without providers. `android-bidirectional-provider-live-e2e` remains blocked on a supported local Anytype sink and the unresolved Rust CLI wire acceptance. No live ContentObserver local-edit ingestion, Anytype sink write, restart/conflict/tombstone round trip, or loop-prevention receipt exists. |
| Tasks.org provider | **Explicitly unsupported in tested runtime** | The clean API-35 probe found no `org.tasks` package and no `org.tasks.api` authority; no task rows or writes were attempted. `TasksOrgAdapter` is version/permission-gated and no-write by default. A pinned Tasks.org 15.12+ read-only capability probe, followed by bounded CRUD only if the exact contract is present, remains unrun. There is no platform-wide Android TasksContract claim. |
| DAVx5/Tasks.org live interoperability | **Explicit authority-gated blocker** | Sanitized DAVx5/Tasks.org-shaped discovery, VTODO CRUD, reconnect, ETag, and error replay passes in `android-dav-client-profile`/`android-davx5-tasks-replay`. No real DAVx5 or Tasks.org APK/account/device run exists; `android-live-validation` and `android-live-interop` remain blocked on disposable client authority and credentials. The project therefore must not claim live-client compatibility. |
| Android account/authenticator lifecycle | **Provider-free and package lifecycle partial** | `android-authenticator-account-lifecycle` tests cover generation, token handoff, cleanup ordering, permission denial, and fail-closed behavior using fakes. Package install/upgrade/restart/uninstall/reinstall preservation was exercised with synthetic rows. Real AccountManager add/remove/re-add through the authenticator and `onPerformSync` recovery were not exercised; the release receipt explicitly says account setup is not implemented. |
| Android backup/recovery and release | **Partial** | `allowBackup=false` is explicit, and release tests preserve synthetic/unrelated provider rows across package lifecycle. This is not a backup/restore implementation or receipt. Current arm64 release packaging, real account lifecycle, credential recovery, and device release validation remain open. |
| Desktop packaging/service integration | **Bounded Linux smoke proven; cross-platform release incomplete** | `packaging/release.sh`, `packaging/install.sh`, `packaging/uninstall.sh`, `scripts/test-packaging.sh`, and CI manifest cover Linux staged artifacts, checksums, safe install/upgrade/uninstall, and token-free config. Windows/macOS native bundles, signing/publication, and platform service integration are explicitly deferred. |

## Current command evidence

The Android provider/DAV/Tasks.org capability closure is summarized in
[`android-capability-closure.md`](android-capability-closure.md). It confirms
that direct ContactsContract and CalendarContract synthetic provider CRUD is
separate from the DAV fallback replay evidence, and that the Tasks.org result
is absence in the tested API-35 image rather than a platform-wide capability
claim. It does not close the live-client or production Anytype-sync gaps.

Commands run during this audit:

```text
env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin \
  cargo test --workspace --offline
  -> exit 101: three app listener tests fail at TcpListener::bind with
     Operation not permitted; other suites shown in the run passed.

env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin \
  cargo fmt --all -- --check
  -> pass

env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin \
  cargo clippy --workspace --all-targets --offline -- -D warnings
  -> pass

env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin \
  cargo test -p any-cal-app --test app --offline
  -> 23 passed, 0 failed

env -u LD_PRELOAD PATH=/home/x/.cargo/bin:/usr/bin:/bin \
  cargo test -p any-cal-anytype-adapter -p any-cal-sync \
    -p any-cal-observability --offline
  -> adapter 15 + HTTP 15 passed (1 HTTPS socket test ignored),
     wire 4, sync 14, observability 12; no failures
```

The full workspace result is therefore not green in this restricted
environment. A host-context socket run is required before promoting the
listener and ignored TLS tests to a complete test gate.

## Smallest remaining evidence-producing order

1. Reconcile the stale Android runtime/release receipts, rebuild the current
   APK through `build-abis.sh`, assert both arm64-v8a and x86_64 are inside the
   same APK, and run the credential-free API-35 install/load probe.
2. Finish the Rust wire DTO against the redacted local CLI response/request
   fixtures, then rerun one fresh disposable adapter CRUD/archive/relist batch
   through the supported guest-local HTTP service. Do not reuse old keys or
   accounts.
3. Replace Android's `NoOpRustSyncBridge` and
   `UnavailableCalendarBridgeSource` with the durable Anytype bridge and secure
   credential handoff. Run one-way Contacts and Calendar sync through the real
   `onPerformSync` path, including checkpoint/restart and account-owned cleanup.
4. Run the host-context DAV listener/TLS certificate matrix and a reverse-proxy
   HTTPS smoke test. Keep the six public coordinator listeners out of the HTTP
   path unless an actual Any-Sync peer transport is implemented.
5. With a working disposable local Anytype sink, run Android observer/local-edit
   bidirectional E2E and conflict/tombstone/restart tests.
6. Install a pinned Tasks.org 15.12+ APK for a read-only capability probe and
   only then request bounded CRUD authority; separately run live DAVx5 only
   with an explicitly disposable client account/APK/device.
7. Complete AccountManager/authenticator lifecycle, backup/recovery policy, and
   current arm64 release/upgrade/uninstall evidence. Add an independent
   credential-shaped-artifact scan and resolve the historical secret-output
   incident before calling secret-safe operations complete.

Until these gates are satisfied, the objective must remain active. The
documented authority-gated Android client blockers are valid blockers, not
evidence of compatibility.
