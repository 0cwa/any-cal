# Anytype live semantics probe

Updated 2026-09-20. This is the redacted result of the fresh disposable
Anytype semantics run. The complete safe-change receipt is
`safe-change-run-anytype-live-semantics.json`.

## Observed

- One marked `page` object was created with the observed `description` text and
  `done` checkbox properties: `201 Created`.
- Direct reads returned `200` immediately and at bounded 1-, 2-, and 5-second
  checks. No `ETag`, revision field, or `Retry-After` was present.
- A normal update returned `200` and was visible on a one-second read.
- A deliberately stale `If-Match` header still returned `200`; conditional
  writes were not observed.
- Two sequential stale writes both returned `200`; the later value won. This is
  an observed last-write-wins risk, not a safe concurrency guarantee.
- A PATCH sent with a one-millisecond client timeout returned curl exit 28 with
  zero response bytes. A single follow-up read showed that the mutation had
  committed. No blind retry was made.
- `DELETE` returned `200`; direct GET still returned the object with
  `archived=true`, while normal relist omitted it.
- No natural `429` or `Retry-After` occurred in this small run.

## Cleanup and gaps

The disposable API key was revoked and the post-revoke listing had zero rows.
The CLI logged out, guest-private state was removed, and the sparse finite-
egress VM was stopped and removed. The pinned CLI/API does not expose a remote
account-deletion operation, so the disposable bot account was abandoned with
no active key; no local account state remains.

Still unverified: rate-limit thresholds/backoff, server revisions or true
conditional writes, parallel interleaving, change streams, reopen/restore,
hard deletion, and live HTTPS/TLS behavior.

## Design consequences

Any-Cal must reconcile timed-out writes by stable object identity and a bounded
read/relist before retrying. It should treat Anytype DELETE as archive/tombstone
plus verification, retain local checkpoints and drift detection, and avoid
claiming optimistic concurrency until a later API version demonstrates a
server-side revision token.
