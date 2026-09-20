# Research and validation register

Items marked **blocking** must be answered before the affected feature is
advertised.  The prototype should fail safely—preserve data and omit an
unsupported capability—rather than guess.

Development workflow, spike boundaries, CI staging, and agent ownership are
recorded in [development-efficiency.md](development-efficiency.md).

## Anytype data model

- **Blocking — structured multi-value storage:** establish the best native
  representation for labelled repeated DAV fields.  The published API schema
  currently shows scalar text/phone/email/url values and arrays only for
  objects/files/multi-selects.  Test current desktop and API behaviour.
- **Blocking — opaque field storage:** test size, newlines, UTF-8, JSON,
  hidden/local properties, API round-trips, concurrent edits, reopen/sync,
  and user edits for `dav_fields`. Until this passes, text/body JSON is only
  a candidate, not canonical storage.
- **Blocking — change detection:** verify modification timestamps, polling
  consistency, archive/delete visibility, rate limits, and any available
  gRPC/event stream. Do not assume a general object-change feed exists.
- **Blocking — mutable type schema:** confirm which API calls safely create
  a property and attach it to an existing Type, and how existing user schema
  changes are detected.  Do not auto-promote arbitrary `X-` fields.
- **Important — linked objects:** test object-array relations, back-links,
  deletion/archival, and whether the API can efficiently filter by them.
- **Important — rich text/files:** test Markdown conversion and the File API
  before claiming vCard photo or iCalendar attachment support.

## EteSync/Etebase integration

- **Blocking — architecture gate:** decide whether native EteSync app support
  justifies a durable Etebase projection.  Strict Anytype-only mode must use a
  direct DAV service or Anytype-backed Radicale storage fork.
- **Blocking — stock adapter boundary:** verify the exact `etesync-dav`
  version and its Etebase-specific Radicale storage assumptions; it is not a
  generic backend interface.  See [etesync-research.md](etesync-research.md).
- **Important — licensing/maintenance:** review GPL-3.0 obligations, current
  release maintenance, Radicale coupling, and whether a fork can be packaged
  cross-platform.

## Android provider source

- **Settled boundary:** Android has no generic built-in DAV endpoint. DAVx⁵
  synchronizes into `ContactsContract`, `CalendarContract`, and app-specific
  task providers; see [android-research.md](android-research.md).
- **Important — existing-account import:** implement a provider-shaped fixture
  first. Record account, raw/provider record IDs, source hashes, ownership,
  and origin. Treat normalized provider data as potentially lossy compared
  with the original DAV document.
- **Blocking — Any-Cal-owned account:** decide whether an Android companion
  sync adapter is worth permissions, account registration, background/Doze
  scheduling, conflict/deletion semantics, and loop prevention.
- **Blocking — companion endpoint:** if desktop Any-Cal must consume phone
  state, design and secure an explicit companion API or DAV endpoint; do not
  assume DAVx⁵ supplies one.
- **Important — task provider:** choose and test an initial provider such as
  OpenTasks/jtx Board before promising task import or bidirectional sync.

## DAV server protocol

- **Blocking — discovery profile:** specify exact URLs, principals,
  home-sets, collections and DAV response properties required by target
  clients.
- **Blocking — write correctness:** implement `GET`, `PUT`, `DELETE`,
  `PROPFIND`, and required `REPORT` variants with `If-Match`/ETag behaviour.
- **Blocking — server foundation:** compare Rust `dav-server`/`mailrs-dav`
  with Go `go-webdav` for actual CardDAV and VTODO coverage before selecting
  the implementation language. See [architecture-research.md](architecture-research.md).
- **Important — bounded comparison:** use one Rust primary spike and a
  disposable Radicale compatibility oracle; only run the Go fallback after a
  concrete Rust stop condition. Avoid a three-way production contest and
  record GPL/MIT licensing implications.
- **Important — incremental sync:** decide whether to implement RFC 6578
  sync tokens or use full enumeration/ETags first.  Never advertise
  `sync-collection` until correct.
- **Important — authentication/TLS:** choose app passwords, credential
  storage, network exposure, certificate provisioning, and a safe deployment
  model for phones outside the local network.

## Client compatibility matrix

Start with these integration tests and record exact request/response traces:

| Area | Primary client | Follow-up |
| --- | --- | --- |
| VTODO | Tasks.org | DAVx5, Thunderbird |
| Contacts | Apple Contacts | Android Contacts through DAVx5, Thunderbird |
| Events | Thunderbird | Apple Calendar, Android calendar client |

For every client test creation, update, delete, offline change, conflict,
unknown extension preservation, collection discovery, and repeated sync.

## Explicitly deferred capabilities

- CalDAV scheduling inbox/outbox, invitation delivery, and free-busy.
- Recurrence expansion/exceptions until tested.
- Alarms, attachments, and photo delivery.
- DAV sharing/ACLs and multi-user permissions.
- Exact preservation of byte ordering/folding in DAV documents; preserve
  semantic values and parameters instead, then serialize valid canonical DAV.
