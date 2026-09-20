# EteSync / Etebase integration research

**Retrieval date:** 2026-08-30

## Important distinction

EteSync's current protocol/platform is Etebase.  Stock
[`etesync-dav`](https://github.com/etesync/etesync-dav) is a local Radicale
CalDAV/CardDAV adapter whose storage implementation is tightly coupled to the
EteSync/Etebase Python API.  It is not a generic DAV server that accepts an
arbitrary database backend over HTTP.

The adapter directly uses Radicale `BaseStorage`/`BaseCollection`, EteSync
collection classes, vObject parsing, a background Etebase synchronizer, and a
local href mapping database.  See the primary
[storage implementation](https://raw.githubusercontent.com/etesync/etesync-dav/master/etesync_dav/radicale/storage.py).
The project is GPL-3.0 and depends on Radicale; review derivative-work and
distribution implications before reusing or forking it.

The EteSync Android app should not be confused with an Android DAV endpoint.
It is an Etebase client with its own account/storage path. Android's generic
contacts and calendar providers are a separate possible source, covered in
[android-research.md](android-research.md). A desktop Any-Cal process cannot
read either provider state or the EteSync app's local state without an Android
companion or an explicitly exposed network service.

## Native Etebase model

Etebase collections have immutable encrypted collection types, collection
sync tokens, revisions, and arbitrary encrypted item content.  The native
types relevant here are:

- `etebase.vcard`: address books; item `name` is the vCard UID and content is
  the complete vCard.
- `etebase.vtodo`: task lists; item `name` is the VTODO UID and content is the
  complete iCalendar VTODO.
- `etebase.vevent`: calendars; item `name` is the VEVENT UID and content is
  the complete iCalendar VEVENT.

Primary sources: [Etebase overview](https://docs.etebase.com/overview),
[vCard type](https://docs.etebase.com/type-specs/address-book),
[VTODO type](https://docs.etebase.com/type-specs/tasks), and
[VEVENT type](https://docs.etebase.com/type-specs/calendar).

This complete-document model can preserve repeated fields, parameters,
unknown properties, and vendor extensions.  It does not remove data-loss risk
in client application providers (Android/Apple/Tasks.org) or in Any-Cal's
projection logic.

## Three supported architectural modes

### 1. Direct Anytype-backed DAV

```text
DAV clients -> Any-Cal DAV service -> Anytype API
```

Anytype is the only durable database.  This is the strict architecture, but
Any-Cal owns DAV interoperability.  It does not automatically make the
EteSync mobile app a client; EteSync apps speak Etebase, not arbitrary DAV.

### 2. Anytype-backed Radicale storage fork or shim

```text
DAV clients -> Radicale/etesync-dav-derived service -> Anytype API
```

Forking the Radicale storage layer and replacing Etebase collections/items
with an Anytype adapter may reduce DAV compatibility work while retaining
Anytype as the sole store.  A Python API shim could prove the idea faster, but
it relies on undocumented internal `etesync-dav` object behavior and is not a
stable product boundary.  This is the recommended strict-mode prototype
experiment.

### 3. Any-Cal Etebase projection

```text
Anytype -> Any-Cal Etebase client/projector -> Etebase
                                         -> etesync-dav / EteSync apps
```

This can reuse stock `etesync-dav` and native EteSync apps if Any-Cal writes
the exact `etebase.vcard`, `etebase.vtodo`, and `etebase.vevent` types.  It
necessarily creates a durable encrypted replica/projection in Etebase; it is
not compatible with a strict interpretation that Anytype is the sole durable
database.  Anytype should remain rebuildable source-of-truth, with complete
wire documents retained there, but Etebase still has independent revisions and
conflict state.

## Explicit architecture gate

The choice must be made per deployment:

| Requirement | Architecture gate |
| --- | --- |
| Anytype must be the only durable store | Mode 1 or 2; do not use Etebase projection |
| Native EteSync Android/iOS app is required | Mode 3; accept encrypted Etebase replica |
| DAV client compatibility is the priority | Start with Mode 2 Radicale storage spike |
| EteSync protocol server is requested | Separate high-risk project; defer |

Implementing an Etebase-compatible server is not a shortcut: Etebase servers
are zero-knowledge encrypted stores and cannot inspect plaintext to map it to
Anytype.  A correct server also needs authentication, encrypted request
formats, signatures, revisions, memberships, pagination, sync tokens, and
conflict behavior.  Defer this mode.

## Data and identity considerations

For Mode 3, maintain stable mappings among Anytype object ID, Etebase item ID,
DAV UID, and collection ID.  Etebase native types require item metadata
`name` to equal the UID in the wire document.  Preserve the complete vCard or
iCalendar content in Anytype; regenerate only the fields intentionally mapped
from visible Anytype properties.

Etebase revisions can provide recovery, but they do not define semantic
ownership between Anytype and a DAV client.  Any-Cal still needs explicit
field conflict, deletion/archive, import, and unknown-field policies.

Stock `etesync-dav` currently has important compatibility caveats: it hashes
UIDs to create hrefs, keeps a local href mapper, transforms vCard 4 to vCard 3
for clients, may alter/drop photos it cannot convert, does not implement
collection creation or MOVE, and has a FIXME for sync-token filtering.  These
must be tested rather than assumed away.

## Decisive experiments

1. Run the exact current `etesync-dav` release with a self-hosted/local Etebase
   server; record collection discovery and Tasks.org, DAVx5, Thunderbird,
   Apple Contacts/Calendar behavior.
2. Inspect/test whether EteSync Android can use a custom server or only an
   Etebase-compatible endpoint.  Treat app compatibility as unproven until a
   real account and collection test succeeds.
3. Build a Radicale storage fork spike: replace only Etebase item/collection
   access with a fake backend, then Anytype API calls. Test CardDAV and VTODO
   before adding VEVENT.
4. Build an Etebase projection spike with all three native collection types;
   test repeated parameterized fields, `X-*` properties, groups, recurrence,
   alarms, deletes, ETags, and round trips through stock `etesync-dav`.
5. Delete the Etebase projection and rebuild it solely from Anytype. Verify
   stable UIDs, no duplicates, preserved opaque fields, and predictable
   collection identity.
6. Create concurrent edits in Anytype and EteSync/DAV clients. Record Etebase
   revision behavior and decide whether a user-visible conflict queue is
   required.

## Provisional recommendation

Prioritize CardDAV and CalDAV VTODO.  For the strict Anytype-only product,
start with a Radicale storage fork (or disposable shim) and keep direct DAV as
the long-term boundary.  Add an optional Etebase projection only if native
EteSync app compatibility justifies a second durable encrypted representation.

## Status for implementation

Settled: direct Anytype-backed CardDAV/VTODO remains the strict primary;
Radicale storage replacement is the fastest compatibility experiment; no
Etebase-compatible server belongs in the MVP. Blocking: verify native app
custom-server behavior and run the projection round-trip before treating
Etebase as a supported mode. Do not let EteSync app compatibility silently
change the single-durable-store invariant of strict mode.
