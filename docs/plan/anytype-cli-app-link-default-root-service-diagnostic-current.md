# Anytype CLI default-root service diagnostic

Updated 2026-09-20. This was a read-only, service-only diagnostic of the
pinned `anytype-cli v0.3.6`. It used one fresh Fedora 42 microsandbox, a
read-only CLI mount, a shared mode-0700 HOME/XDG wrapper, no `DATA_PATH`, no
explicit root override, and no network. Authentication, API-key operations,
object/Space/schema operations, and provider operations were not invoked.

## Result

The service process remained alive but did not expose loopback HTTP port 31012
within a ten-second readiness window. The guest-private log was reduced before
leaving the guest: 246 bytes, four lines, maximum line length 83, zero empty
lines, zero control bytes, and zero ANSI markers. The process inherited the
expected HOME/XDG identity (`PATH_IDENTITY_EQUAL=1`), and `DATA_PATH` was
absent. Only a salted structural hash was retained.

The receipt is
[`safe-change-run-anytype-cli-app-link-default-root-service-diagnostic-20260920.json`](safe-change-run-anytype-cli-app-link-default-root-service-diagnostic-20260920.json).

## Interpretation and boundary

This confirms the earlier symptom in a smaller, no-network experiment: the
service can remain alive while HTTP readiness is absent. It does not identify
the cause. In particular, the result cannot distinguish network-dependent
initialization from local default-root initialization. It is not evidence of
an app-link, credential, parser, or API incompatibility.

The guest service and state were removed and independently verified absent.
No raw log, path, identifier, credential, or account state was retained.

If further diagnosis is worth the cost, the next bounded lane is the same
service-only test with the exact previously approved coordinator allowlist,
still stopping before authentication or any write. That comparison is the
minimum needed to separate network startup from default-root state behavior.
