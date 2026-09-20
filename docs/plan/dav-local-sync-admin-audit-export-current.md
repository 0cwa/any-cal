# Local DAV sync admin audit/export — current evidence

Date: 2026-09-20

## Scope and boundary

This receipt covers provider-free `App::fake` requests and observability
tests using disposable synthetic checkpoint and audit state. It did not read
or modify credential artifacts, use Anytype, contact a network or external
client, use Android/provider state, use DAVx5/Tasks.org, or use the personal
Flatpak. No automatic retention/rotation/deletion policy was introduced.

## Reproduced defect and focused fix

Admin sync requests previously added `recovery.admin` events only to the
in-memory event buffer. Consequently, capability, denial, malformed-request,
export, restore, and failure outcomes were absent from the durable audit
journal even when `audit_directory` was configured.

`AppGeneric::record_admin_audit` now appends the same bounded event to the
configured `AuditEventWriter` while retaining the in-memory event. The helper
does not alter the admin operation's result when the diagnostic sink is
degraded; writer health exposes append failure separately. Authorization
failures on `/admin/sync*` are recorded before the generic denial response.
Messages contain only operation/status classes and bounded correlation; they
do not contain credentials, paths, payloads, Space IDs, or resource IDs.

## Focused evidence

Commands ran with dynamic loader injection unset and offline:

```text
env -u LD_PRELOAD cargo fmt --all -- --check                         PASS
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo test -p any-cal-app --test app \
  sync_admin_operations_are_persisted_as_bounded_audit_events --offline \
  -- --exact                                                        PASS
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-service-export-target \
  cargo clippy -p any-cal-app --test app --offline -- -D warnings    PASS
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-audit-writer \
  cargo test -p any-cal-observability --offline                      PASS: 24 passed
```

The focused admin suite also passed individually for authenticated export/
restore atomicity, admin health correlation and method handling, audit health
visibility/readiness, required-audit failure, optional-audit recovery, and
restart-safe audit/checkpoint ordering. The new regression test verified
durable records for capability success, export success, malformed restore,
and unauthorized denial, with correlation retained and synthetic token/path
sentinels absent.

## Contract result

- Capability, export, restore, denial, malformed-request, unavailable
  checkpoint, and operation-failure paths now emit typed bounded
  `recovery.admin` events when durable audit is enabled.
- Audit persistence remains atomic, private, deterministic, restart-safe, and
  bounded by the existing writer/store contract; observability tests cover
  queue, checkpoint, corruption, recovery, redaction, and single-writer
  behavior.
- Admin operation responses remain independent of optional audit append
  failures; required DAV-write behavior remains unchanged.

Automatic retention, production telemetry, multi-user policy, live Anytype,
external DAV compatibility, and Android/provider validation remain outside
this local unit.
