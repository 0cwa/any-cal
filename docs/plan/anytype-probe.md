# Anytype adapter probe

The adapter is pinned to the `2025-11-08` API contract (`API_VERSION` in
`crates/anytype-adapter`). The std-only handwritten HTTP transport uses an
injectable exchange seam for deterministic tests and a `TcpStream` in
production. It sends JSON with fixed `Content-Length`,
`Anytype-Version: 2025-11-08`, and optional bearer authorization. The current guarded assumptions are
`/v1/spaces/{space}/objects`, object `GET`/`PATCH`/`DELETE`, with archive also
represented by the documented object `DELETE`; pagination uses numeric
`offset` plus `limit` and a response continuation offset. The exact API
header is `Anytype-Version: 2025-11-08` (no legacy `X-Anytype-Version`).
The headless endpoint default is `http://127.0.0.1:31012`; Anytype's desktop/
MCP endpoint `31009` is not assumed to expose this API. Sanitized response
fixtures are in `fixtures/anytype-api/`.

## Isolated probe runner

`tools/anytype-probe` is a standalone, std-only guarded launcher and bounded
read-only HTTP probe. Run
`cargo run --manifest-path tools/anytype-probe/Cargo.toml --offline --
--api-url http://127.0.0.1:31012 --space-id DISPOSABLE_SPACE --run` for a
read-only API report; supply `ANY_CAL_API_KEY` (or pipe a key with
`--api-key-stdin`). The endpoint must resolve to loopback. It sends
`Anytype-Version: 2025-11-08` bearer-authenticated GETs for space, types,
objects, and properties with bounded response handling, and emits only
redacted summaries plus true SHA-256 body hashes. Supplying
`--binary PATH --run` launches only the external binary's `--version` stage
with an isolated state-root and a reserved localhost port; Flatpak paths are
rejected. API keys are accepted only through the environment or guest stdin
and are never included in arguments or reports. Child stdout/stderr and API
response summaries pass through a secret-aware capture boundary before report
formatting. The boundary removes known full and partial sentinel values and
rejects Authorization/Bearer markers that remain after redaction. Write probing is currently fail-closed and
not implemented; `--allow-write` plus `ANY_CAL_CONFIRM_WRITE=YES` are required,
as are an existing private account app-link file (`--app-link`) and a private
`revoke=available` capability receipt (`--revoke-capability`). Missing or
invalid prerequisites fail before any future key-creation path. This repository
never runs the write stage. State is
preserved by default and cleanup requires an explicit
state root must name a new child under the system temporary directory; a
sentinel prevents cleanup of arbitrary existing paths. Captures are bounded,
redacted, and accompanied by hashes rather than raw credential-bearing
fixtures. The probe state root is created with mode `0700`, its ownership
sentinel with mode `0600` on Unix, and `--cleanup` verifies that the root is
absent after removal.

The probe does not create accounts or API keys, so it cannot claim revocation
evidence. Account/key creation must remain in a separately isolated guest
workflow that captures CLI output before any host-visible logging, verifies
revocation through the CLI/API, and destroys the guest state. The synthetic
sentinel tests in `tools/anytype-probe` cover full keys, key fragments, and
Bearer/Authorization markers without storing real credentials.

The pinned CLI v0.3.6 API-key create command has a narrow textual handoff:
`API key created successfully`, `Name: ...`, and `Key: ...`. The guest-only
parser accepts exactly those fields, returns only field-presence booleans, and
never returns the key value. App IDs are obtained from the actual
`auth apikey list` table, whose columns are `NAME`, `ID`, `KEY`, and `CREATED`.
The parser accepts exactly one row, requires the CLI's shortened `8-chars...`
key display, and returns only a SHA-256 fingerprint handle. Duplicate rows,
unexpected columns, malformed timestamps, and full or unshortened key values
fail closed. The source's exact no-row marker, `No API keys found.`, is
recognized as an explicit empty result and produces no handle. Redacted
synthetic fixtures are under
`tools/anytype-probe/fixtures/`.

`AnytypeRepository` currently treats the core repository cache as the stable
DAV view and pushes canonical envelopes through the injected transport. The
opaque envelope is stored in the object body; visible identity properties are
stored separately. Storage mode (text property, body, file, or linked child)
remains deliberately pluggable and is not selected without a live probe.

The bounded live CRUD probe completed on 2026-09-20 in a fresh finite-egress
microVM, using a fresh disposable account/key and two uniquely marked `page`
objects. Create/read/PATCH/read-after-update/archive/relist succeeded; the
normal relist returned zero objects after archive. The redacted receipt is
`docs/plan/safe-change-run-anytype-crud-20260920.json`.

This does not prove live concurrent writes, rate-limit/retry behavior,
unknown-outcome idempotency, ETag/revision semantics, change streams, or
reopen/sync behavior. Those remain separate bounded probes using fresh
credentials and newly approved safe-change artifacts.

Unsupported until evidence exists: arbitrary property type fidelity, conflict
resolution, webhook/change-stream semantics, and live API rate-limit behavior.

The executable HTTP proof is deliberately local and transport-neutral:
`crates/anytype-adapter/tests/http_transport.rs` drives the real request
builder/parser through a scripted exchange and covers exact paths, headers,
framing, pagination, CRUD/archive, typed status failures, and malformed,
truncated, oversized, and wrong-content-type responses. The app exposes the
same seam through `http_with_exchange` for display-independent HTTP-mode
health/check tests. This is not evidence that a live Anytype server accepts
these paths or that HTTPS/TLS, retries, rate-limit backoff, schema discovery,
or token scopes work; those remain probe work. Before enabling writes, run a
read-only disposable-space probe, then explicitly authorize a bounded create,
PATCH, archive/delete, and reopen/list probe; retain request/response fixtures
with secrets removed. Live schema, pagination field names, tombstone shape,
and token scopes remain unverified here.

## Fake-boundary read policy

The fake transport now exercises the read boundary without implying live API
compatibility. Writes use the opaque canonical envelope plus deterministic
properties: `dav_uid`, `dav_kind`, and `dav.property.<field>.<occurrence>`.
Repeated values retain their occurrence index; unknown DAV fields remain in
the envelope. Only Contact and Task objects are projected into the current DAV
view; archived objects and Event/ContactGroup objects are hidden.

Remote failures retain their transport category (`not_found`, `conflict`,
`auth`, `forbidden`, `rate_limited`, `timeout`, `malformed`,
`delayed_visibility`, `invalid_request`, `unavailable`, or `other`). The
current core repository error enum can represent only a subset, so the adapter
maps unsupported categories conservatively and exposes the original category
at the transport boundary. A future core error extension is required before
claiming end-to-end preservation of auth and rate-limit semantics.
