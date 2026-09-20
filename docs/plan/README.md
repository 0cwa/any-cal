# Anytype DAV adapter plan

## Purpose

Any-Cal is an always-on CardDAV and CalDAV server whose sole durable data
store is an Anytype Space. It translates DAV resources to Anytype objects and
translates Anytype objects back to vCard/iCalendar on request. The current
desktop client is a thin Slint setup/status/diagnostics UI; it is not the DAV
implementation. Tauri 2 + Svelte/TypeScript was researched as an alternative,
but is not a dependency or supported build target in this repository.

The service defaults to the Anytype HTTP transport. A listener is only local
configuration readiness: `/health` reports whether the upstream has been
tested, while `/ready` performs a bounded Space read and stays `503` until the
live transport succeeds. The fake transport is explicit fixture/development
mode and is never a production fallback.

Contacts are exposed through CardDAV. Tasks are exposed as CalDAV `VTODO`
resources. Calendar events are reserved for a later `VEVENT` profile.

## Non-negotiable invariants

- No separate persistent sync database.  A short-lived in-memory cache is
  permitted, but all recoverable state lives in Anytype.
- An Anytype object is the canonical resource.  Its immutable Anytype ID
  determines the DAV resource URL; DAV `UID` is retained as object data.
- A complete, structured representation of unprojected DAV data is intended
  to be stored in the same object. The storage mechanism is a blocking,
  unproven experiment until it prevents loss of parameters and unknown/vendor
  fields in real Anytype round-trips.
- Normal Anytype properties are a useful, user-editable projection of DAV
  data, not a lossy replacement for it.
- The server advertises only capabilities it implements and tests against
  real clients.

## Target architecture

```text
DAV client (Tasks.org, Contacts app, Thunderbird, ...)
       | HTTPS + DAV authentication
       v
Any-Cal server (Rust headless service)
  - DAV discovery and WebDAV methods
  - CardDAV/CalDAV representations
  - mapping and validation
       | local or headless Anytype API
       v
Anytype Space (only durable store)

Slint UI (current thin client): setup, server status, and local health
diagnostics
```

See [schemas.md](schemas.md) for the object models,
[development-efficiency.md](development-efficiency.md) for thin-slice sequencing,
agent ownership, and automation,
[research.md](research.md) for data/protocol decisions,
[android-research.md](android-research.md) for the Android provider-source
boundary,
[etesync-research.md](etesync-research.md) for EteSync/Etebase modes and
[architecture-research.md](architecture-research.md) for language, server,
UI, and bakeoff decisions.

## Complete enough to implement

The direct CardDAV + CalDAV VTODO slice is now specified well enough to
implement. Android provider import, Etebase projection, VEVENT, recurrence,
and richer mapping UI are extension seams, not prerequisites. A decision remains
blocking only where the linked research register says so; the prototype must preserve
unknown data and decline to advertise unsupported capabilities.

## Phased delivery

1. Foundation: connect to Anytype, choose the strict Anytype-only or optional
   Etebase-projection architecture gate, create/validate schemas, authenticate DAV
   clients, and implement DAV discovery plus collection listing.
2. CardDAV MVP: one Contact collection, `GET`/`PUT`/`DELETE`, `PROPFIND`,
   ETags, contact and group mappings, and client tests.
3. CalDAV VTODO MVP: Task List collections, the core VTODO fields, task
   project links, and Tasks.org tests.
4. CalDAV VEVENT MVP: core events and date-range queries.
5. Android provider import fixture and, only if justified, an Android
   companion/sync adapter.
6. Incremental sync, recurrence, alarms, attachments, richer group/client
   interop, and optional sharing.

Each phase ends with interoperability tests before expanding the advertised
DAV feature set.
