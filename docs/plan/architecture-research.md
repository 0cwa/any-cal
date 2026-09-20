# Architecture and implementation research

## Current implementation status

The repository now implements the headless service and desktop shell in Rust.
`crates/ui` is a Slint client of the local health/configuration boundary. Tauri
2 + Svelte/TypeScript and Go remain documented alternatives from the original
bakeoff research; neither is a dependency or a supported build target here.
The current release matrix is Rust desktop binaries plus the Android
foundation build, not a claim that every researched option is production
ready.

## Decision scope

EteSync/Etebase is an optional deployment concern, not evidence that Anytype
can be replaced as the canonical store.  See
[etesync-research.md](etesync-research.md) for the explicit architecture gate.

Choose the implementation language, DAV server foundation, and desktop UI
architecture for an Anytype-backed CardDAV/CalDAV server. Anytype remains the
only durable data store; the application must also run as an always-on
headless service. The first protocol target is CardDAV contacts plus CalDAV
`VTODO` tasks. `VEVENT`, recurrence, alarms, sharing, and scheduling follow
later.

The original decision was deliberately not final until a Rust/Go vertical-slice
bakeoff could be run against real clients. The current repository has chosen
the Rust implementation for the executable slice; the alternatives below are
kept as fallback research, not parallel product targets.

The development-efficiency rules are collected in
[development-efficiency.md](development-efficiency.md). The first commitment
is a semantic, language-neutral repository/resource contract; generic DAV ASTs,
mapping DSLs, and generated-client sprawl are deferred until fixture evidence
requires them.

## Options and evidence

### Rust core/server + Tauri 2/Svelte UI (researched alternative)

Rust provides a shared core for the DAV model, Anytype adapter, and desktop
application. [`dav-server`](https://docs.rs/dav-server/latest/dav_server/)
provides an async WebDAV handler with custom backend traits,
ETags/preconditions, locks, and documented CalDAV `MKCALENDAR` and `REPORT`
support. Its documentation exposes CardDAV modules, but CardDAV coverage must
be verified directly.

[`mailrs-dav`](https://docs.rs/mailrs-dav/latest/mailrs_dav/) advertises
framework-agnostic CalDAV/CardDAV handlers and
`CalendarStore`/`AddressBookStore` traits, but its maturity and client
compatibility require testing. The [`anytype` Rust crate](https://docs.rs/anytype/latest/anytype/)
offers async operations, pagination, caching, authentication/keyring support,
and HTTP/gRPC coverage, but is not the official Anytype SDK. The official
[Anytype API](https://developers.anytype.io/docs/reference/2025-04-22/anytype-api/)
also publishes OpenAPI and a headless CLI, so generated HTTP bindings remain a
fallback.

### Rust core/server + Slint UI

[Slint](https://docs.slint.dev/latest/docs/slint/guide/platforms/desktop/)
supports Windows, macOS, and Linux with Rust bindings and a native declarative
UI. It keeps the process and language model simple and can be smaller than a
WebView shell. The tradeoff is less availability of ready-made form/table,
accessibility, and browser-testing patterns for this configuration-heavy UI;
this is an engineering inference that needs a UI spike.

### Go server + Tauri/Svelte UI

[`emersion/go-webdav`](https://pkg.go.dev/github.com/emersion/go-webdav) is the
strongest server-only candidate found: it explicitly includes WebDAV, CalDAV,
and CardDAV client/server packages, principal support, backend interfaces,
conditional ETag matching, and an MIT license. Go also has straightforward
HTTP/TLS/concurrency and static cross-compilation. Pairing it with Tauri,
however, creates a Rust UI plus Go sidecar: two models, two build pipelines,
lifecycle/IPC handling, and no direct reuse of the Rust core. It should win
only if the Rust server slice fails protocol compatibility or needs materially
more custom code.

### TypeScript/Node and Python

Mozilla's [`ical.js`](https://github.com/kewisch/ical.js) is a strong parser
for iCalendar, jCal, vCard, and jCard, including recurrence tooling and
validation, but no equally compelling maintained full DAV server foundation
was identified. Python's [`vobject`](https://vobject.readthedocs.io/latest/)
is useful for parsing/generation, while
[`python-caldav`](https://github.com/python-caldav/caldav) is primarily a
client. Both are good mapping/test tools, not current product-server choices.

## Debate and disagreements

- Go has the clearest explicitly CardDAV + CalDAV server package; Rust has the
  stronger shared-core and native integration story. The bakeoff decides
  whether Go's protocol advantage is substantial in practice.
- Tauri + Svelte is likely faster for forms, tables, logs, and browser-based
  acceptance tests. Slint remains simpler for a single native Rust binary.
- A hidden JSON DAV envelope may be semantically lossless, but Anytype does
  not document a generic structured-list property. Text/body storage could be
  user-edited or transformed; it is unproven until round-trip tests pass.
- Anytype's current changelog documents API additions, not a general object
  change feed. Polling, timestamps, or gRPC capabilities must be verified
  before ETag/sync assumptions are made. See the [API changelog](https://developers.anytype.io/docs/reference/changelog/).

## Current recommendation

For strict Anytype-only deployments, prototype a Radicale storage fork or
disposable shim backed by Anytype.  An Any-Cal Etebase projection is a separate
mode that improves EteSync-app compatibility but creates a durable encrypted
replica.  Do not implement an Etebase-compatible server in the initial scope.

Use a protocol-independent Rust core and keep the DAV server behind an adapter
boundary:

```text
any-cal-core       Anytype adapter, envelope, mappings, ETags, capabilities
any-cal-server     WebDAV/CardDAV/CalDAV HTTP implementation
any-cal-ui         Slint (current thin desktop client)
```

Evaluate `dav-server` and `mailrs-dav` within the Rust implementation. Keep Go
`go-webdav` and Tauri/Svelte as explicit fallbacks if real-client evidence
shows a material protocol or UI gap; do not add either stack speculatively.

## Complete-enough architecture decision

The implementation boundary is fixed for the current Rust + Slint baseline:

```text
any-cal-core
├── DAV model, parser/serializer, projections, envelope, ETags
├── Anytype repository (strict canonical store)
├── optional Android-provider repository
└── optional Etebase projection repository
any-cal-dav (headless HTTP service)
any-cal-ui (Slint shell; richer mapping views remain an extension)
```

Direct Anytype-backed DAV is the primary architecture. Android is not a
first-class desktop account in the MVP: Android has no generic built-in DAV
endpoint, and DAVx⁵ writes to local providers rather than exposing its own
database. See [android-research.md](android-research.md). Existing-account
Android import is the first Android seam; an Any-Cal-owned Android sync adapter
or a hypothetical companion DAV endpoint comes later.

The fastest protocol compatibility experiment remains a Radicale storage
fork/shim backed by Anytype. Etebase projection remains optional and creates a
second durable encrypted representation; an Etebase-compatible server is not
an MVP target. Rust is the current implementation choice and Slint is the
current desktop shell. Go and Tauri/Svelte remain fallback options if measured
client evidence justifies revisiting them.

The slice is complete enough to implement when Anytype envelope storage,
change detection, CardDAV discovery/report behavior, and Tasks.org VTODO
round-trips have passing evidence.

## P0 experiments

1. Canonical Anytype storage: compare opaque text property, body/code block,
   file payload, and a system-managed linked child object. Test exact UTF-8,
   newline/JSON round-trip, size limits, malformed recovery, reopen/sync,
   concurrent writes, and user edits.
2. Change detection: verify modification timestamps, list/search consistency,
   archive/delete visibility, rate limits, and any available gRPC/event stream
   in desktop and headless Anytype.
3. Rust/Go DAV backend bakeoff with the same minimal CardDAV + VTODO behavior.
4. Rust CardDAV coverage: discovery, home sets, `addressbook-query`,
   `addressbook-multiget`, PUT/DELETE, and ETags.

## P1 experiments

- Extend the Slint mapping/status client for repeated DAV-field editing,
  request logs, and error states; revisit Tauri/Svelte only if a measured
  accessibility or UI-testing gap blocks the current shell.
- Test vCard groups, `RELATED`, labelled values, photos, and unknown `X-`
  fields.
- Test VTODO list membership, subtasks, tags, completion, offline edits,
  conflicts, recurrence/alarm preservation, and vendor fields.
- Test package/startup/shutdown in headless, tray, and desktop modes.

## Concrete 2–3 day vertical slice

Run the Rust spike first with an in-memory repository. Use stock Radicale only
as a disposable compatibility oracle; invoke Go `go-webdav` only after a
concrete Rust stop condition. Do not run three production implementations
concurrently.

### Day 1: shared model and Anytype fake

Define `Collection`, `Resource`, `ResourceKind`, `CanonicalDocument`,
`AnytypeObjectId`, `DavUid`, and `ETag` in a small Rust core. Implement an
in-memory repository with Contact and Task objects plus deterministic vCard/
VTODO parsing, projection, envelope merge, and serialization. Add a
disposable Anytype API fixture for create/read/update and malformed recovery.

### Day 2: two server implementations

Implement one CardDAV address book and one CalDAV VTODO collection with
discovery, `PROPFIND`, `GET`, `PUT`, `DELETE`, relevant `REPORT` multiget/query,
`If-Match`, `If-None-Match`, and deterministic ETags. Build the Rust version
with `dav-server` or `mailrs-dav`, and an equivalent Go version with
`go-webdav`.

### Day 3: client acceptance and decision

Connect Tasks.org and DAVx5 to both servers, and Thunderbird or another
CardDAV client for contacts. Create, update, delete, move between lists, go
offline, reconnect, and preserve unknown fields. Capture traces, custom
protocol code, failures, and recovery behavior. Choose Rust if compatibility
is comparable and shared-core/UI benefits hold; choose Go if it eliminates
substantial protocol work or fixes client failures.

## Acceptance criteria

- DAV-created contacts and tasks create exactly one stable Anytype object.
- Anytype projection edits and DAV edits round-trip in both directions.
- Repeated labelled values, parameters, unknown fields, and vendor fields are
  preserved semantically in the envelope.
- ETags are deterministic; malformed writes do not replace the last valid
  object; restart does not duplicate resources.
- Tasks.org can discover, create, edit, complete, delete, and resynchronize a
  VTODO. A CardDAV client can discover, create, edit, and delete contacts.
- Deletes follow an explicit Anytype archive/tombstone policy.
- Unsupported capabilities are not advertised and secrets/content are absent
  from normal logs.
- An Android provider-shaped fixture demonstrates import identity, ownership,
  permissions, and fields that are necessarily lossy before an Android app is
  attempted.
- Reuse/fork decisions document GPL obligations for Radicale, DAVx⁵, or
  `etesync-dav`; permissive core code does not silently absorb GPL code.

## Revisit gate

The Rust + Slint choice is the current implementation baseline. Reopen the
language/UI decision only after measured client-compatibility or accessibility
evidence shows that the current stack cannot satisfy a required target; do not
maintain multiple production implementations in parallel.
