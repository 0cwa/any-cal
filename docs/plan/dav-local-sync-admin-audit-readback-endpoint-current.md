# Local sync-admin audit readback endpoint

Status: complete for the provider-free synthetic scope of
`dav-local-sync-admin-audit-readback-endpoint`.

Date: 2026-09-20

## Contract

`GET /admin/sync/audit` is an operator-only extension of the existing local
`/admin/sync` boundary. It requires the configured admin credential, rejects
request bodies, and accepts only the bounded query parameters `limit` and
`after` (sequence number). `limit` is in the inclusive range 1..64; the
default is 64. Results are deterministic sequence order and return only:

- sequence and timestamp;
- bounded event kind/category;
- allow-listed `operation` and `status` tokens extracted from the already
  redacted message; and
- a truncated SHA-256 digest of the first available correlation value.

Raw messages, resource paths/identifiers, credentials, and audit-store paths
are not returned. The response is `Cache-Control: no-store`, includes the
existing bounded correlation header, and reports `next_after` as the last
returned sequence.

The endpoint reads through `AuditEventWriter::readback`, which delegates to
the validated durable store. Sequence gaps, duplicate sequences, malformed
records, unavailable stores, and worker failures therefore fail closed with a
bounded `503 audit_unavailable` response. The admin credential boundary is
separate from ordinary DAV/health authorization behavior.

## Evidence

- `env -u LD_PRELOAD cargo test -p any-cal-app --test app audit -- --nocapture`:
  9 passed, including authentication separation, bounded query rejection,
  redaction, correlation digest, request-body rejection, and corruption to
  `503` behavior.
- `env -u LD_PRELOAD cargo test -p any-cal-observability --lib`: 26 passed,
  including sequence/corruption/restart/readback-store failure coverage.
- `env -u LD_PRELOAD cargo clippy -p any-cal-observability --all-targets -- -D warnings`:
  passed.
- `env -u LD_PRELOAD cargo fmt --all -- --check`: passed.

An app Clippy retry using a fresh target directory was unable to finish because
the shared temporary filesystem reached its quota while compiling existing
workspace dependencies; no source diagnostic was produced. The focused app
test and observability Clippy checks passed.

## Scope boundaries

All records and credentials used by the test were synthetic in-memory or
disposable temporary state. No live Anytype service, network, external DAV
client, Android/provider state, DAVx5/Tasks.org, personal Flatpak, pending
credential artifact, raw audit export, or automatic retention policy was
accessed.
