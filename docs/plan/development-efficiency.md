# Development efficiency and rework prevention

## Decision

Commit first to a semantic, language-neutral repository/resource contract. Do
not commit yet to a generic DAV AST, mapping DSL, or generated server/client
stack. The first implementation should hard-code the small Contact + VTODO
profile, while preserving a clean seam for later VEVENT, Android, and Etebase
connectors.

The fastest learning path is one Rust headless server spike using
`dav-server`, with an in-memory repository. Use stock Radicale as a disposable
compatibility oracle and run Go `go-webdav` only as a bounded fallback. Do not
run three production implementations in parallel.

## Thin slice and gates

The first complete user-visible outcome is one CardDAV address book and one
CalDAV VTODO list usable by a DAV client. It includes discovery,
`PROPFIND`/`GET`/`PUT`/`DELETE`, relevant query/multiget reports, ETags and
conditional writes, deterministic versioned envelopes, and unknown-field
round-trip tests. Tasks.org plus one CardDAV client are the acceptance clients.

Before live Anytype, pass all fixture tests against an in-memory repository.
Then run one disposable live Anytype probe for create/read/update/archive,
envelope size/UTF-8/newlines, and change detection. Do not build the UI or
Android/Etebase connectors before these gates pass.

Stop and invoke the Go fallback only if Rust requires a substantial protocol
fork, fails required CardDAV/VTODO behavior after a bounded adapter effort, or
cannot keep protocol details out of the repository/mapping layers. Radicale is
GPLv3 and should remain a test oracle/container unless licensing is explicitly
accepted; `go-webdav` is MIT. Record all dependency licenses before packaging.

## Contracts and data handling

Use a repository interface independent of HTTP, Anytype, or a particular
language. Keep a handwritten Anytype adapter behind a pinned/generated
OpenAPI-client boundary. Generate only API boilerplate and mapping manifests;
avoid premature generic DSL/RFC AST/generated-client sprawl.

The envelope is versioned and deterministic. Projection tests must prove that
visible Contact/Task properties merge without deleting repeated, parameterized,
unknown, or vendor fields. Add golden fixtures, property-based normalization,
protocol replay, and a failure-injection fake (timeouts, malformed responses,
conflicts, archive failures). A derived in-memory index may accelerate lookups,
but reconciliation must rebuild it from Anytype after restart; it is never
durable truth.

## Agent ownership and automation

Give agents narrow ownership: model/envelope, mapping, repository fake,
DAV adapter, Anytype probe, fixtures/client traces, and (later) UI. Model and
protocol ownership must not be combined in an unreviewed lane. Each lane must
add an executable contract or fixture and list assumptions/deferrals.

Minimum commands:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask fixture-check
cargo xtask protocol-replay
```

CI should run unit/fixture checks on every change, protocol replay plus a
booted-service job on pull requests, and Radicale/live-Anytype/package smoke
tests nightly or at release. Keep the OS packaging matrix out of ordinary
protocol iterations.

## UI and integration timing

Expose a small authenticated localhost control/status API from the headless
service. The current Slint client consumes this API, keeping server, CLI, tests,
and desktop packaging on one data plane. Revisit Tauri/Svelte only if a
measured mapping-form, accessibility, or browser-test gap blocks the Slint
client.

Android provider and EteSync work starts with sanitized provider-shaped and
Etebase-shaped fixtures, including source IDs, ownership, hashes, and lossy
fields. A live device sync adapter or Etebase-compatible server requires a
separate architecture decision.

## Relative options

| Option | Value | Effort/risk | Decision |
| --- | --- | --- | --- |
| Rust + fake repository + DAV fixtures | High learning and direct value | Low, reversible | Primary |
| Rust + live Anytype + UI immediately | Medium | High uncertainty | Defer |
| Go and Rust production spikes in parallel | Medium | High coordination cost | Avoid |
| Python/Radicale implementation | High compatibility signal | GPL/integration risk | Oracle only |
| Etebase/Android first | Unclear | High and less reversible | Defer |

The next action is to implement the repository contract, in-memory Contact
and VTODO resources, and the first golden/replay fixtures. The next decision
gate is client compatibility, not UI framework selection.
