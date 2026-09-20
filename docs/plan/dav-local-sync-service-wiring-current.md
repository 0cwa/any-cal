# DAV local sync service wiring

Status: complete for the provider-free local service boundary.

Scope was intentionally limited to the in-process DAV service, the fake
Anytype repository transport, and disposable checkpoint files. No Anytype
credential, live sink, external client, Android provider, DAVx5/Tasks.org
client, personal Flatpak, or pending credential artifact was accessed.

## Verified lifecycle

- `App::with_transport` validates the service configuration, creates the
  configured Contacts and Tasks collections, opens the optional durable sync
  checkpoint, and refuses startup on invalid checkpoint or audit state.
- `PUT` and `DELETE` mutations go through the repository-backed DAV server and
  checkpoint the complete visible collection state before returning success.
  A checkpoint failure changes the response to a bounded recovery error and
  removes the success `ETag` rather than publishing uncheckpointed state.
- Reopening the app with the same checkpoint and fake repository restores
  observed identities and tombstones. Normal observation does not resurrect a
  tombstoned identity; explicit resurrection remains required.
- Dropping and reopening the app was exercised as the local shutdown/restart
  boundary. The restart readiness response remains truthful and no request
  event includes a resource identifier or credential.

## Verified boundaries

- `/health`, `/status`, and `/ready` are GET-only, bounded, uncachable
  diagnostics. Required-audit failure makes `/ready` return `503` and does not
  permit DAV mutation to proceed as healthy.
- DAV authentication, LAN policy, method/capability authorization, rate
  limiting, conditional writes, stale writes, malformed writes, and transport
  failures were exercised. Unauthorized requests use generic responses and do
  not consume the authenticated rate budget.
- Request and sync correlation uses bounded `X-Request-ID`/`X-Sync-ID` values;
  diagnostics expose only categories, counts, and statuses. Tests assert that
  Anytype identifiers, bearer sentinels, credentials, and sensitive paths do
  not appear in response bodies or events.

## Evidence

Using the current pinned workspace toolchain with `LD_PRELOAD` unset and a
fresh target directory:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-wiring-target-20260920 \
  cargo clippy -p any-cal-app --tests --offline -- -D warnings
  passed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-wiring-target-20260920 \
  cargo test -p any-cal-app --test app --test local_sync_mutation --offline
  43 passed; 0 failed
```

The focused suite covers 41 application lifecycle/boundary tests and 2
provider-free mutation/restart tests. Workspace listener tests that require
host socket privileges were not used as evidence for this local in-process
lane.

No implementation defect was reproduced, so no production source change was
needed in this unit. Remaining live Anytype and external-client validation is
separately gated and is not claimed here.
