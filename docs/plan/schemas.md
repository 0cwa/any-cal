# Preliminary Anytype schemas

## Shared DAV envelope (candidate; not yet proven)

The envelope is a versioned, deterministic implementation contract. The first
Contact and Task mappings are intentionally explicit; do not introduce a
generic mapping DSL until fixture evidence shows repeated evolution requires
it. A derived index may be rebuilt from Anytype after restart, but is not
durable schema state.

If Etebase projection mode is enabled, the envelope also needs the Etebase
collection/item IDs and native type (`etebase.vcard`, `etebase.vtodo`, or
`etebase.vevent`).  This is projection metadata, not permission to treat
Etebase as the canonical CRM database.  See [etesync-research.md](etesync-research.md).

Every DAV-backed object has these hidden/internal properties.  They are
stored in Anytype, not an external database.

Objects imported from Android additionally need source metadata. This is
provider identity, not a replacement for `dav_uid` or the canonical DAV
envelope:

```text
source_kind       android_contacts | android_calendar | android_tasks
source_account    Android account/sync-adapter account name
source_record_id  provider row or raw-contact ID
source_hash       normalized provider projection hash
origin            any-cal | external | imported
ownership         import-only | android-owned | any-cal-owned
```

Android provider values are normalized and may be lossy; do not claim raw
vCard/iCalendar round-trip fidelity for this source mode until fixture and
device tests pass. See [android-research.md](android-research.md).

| Property | Format | Purpose |
| --- | --- | --- |
| `dav_kind` | select | `contact`, `contact_group`, `task`, or `event` |
| `dav_uid` | text | Stable vCard/iCalendar UID |
| `dav_fields` | text / structured JSON | All property occurrences not represented losslessly by ordinary fields |
| `dav_version` | select | `vcard-3`, `vcard-4`, or `ical-2` |
| `dav_deleted_at` | date | Optional tombstone/audit data |

`dav_fields` is a structured list of property occurrences, not a flat
key/value map.  This preserves repeated values and their individual
parameters:

```json
{
  "TEL": [
    {"value": "+461234", "params": {"TYPE": ["cell", "voice"], "PREF": ["1"]}}
  ],
  "X-CLIENT-FIELD": [{"value": "kept intact", "params": {}}]
}
```

The server merges the visible property projection with this structure when
serving DAV.  It must never discard unknown fields during an update.

**[RESEARCH — P0]** Confirm Anytype's practical text-property size, multiline
JSON behaviour, user-edit/reopen/sync behaviour, and whether a private/local
property can be created and maintained reliably through the public API. Also
compare body/code block, file payload, and a system-managed linked child
object. No candidate is canonical until this test passes; see
[architecture-research.md](architecture-research.md).

## Cross-Space materialized references and personal facets

Cross-Space composition does not create another canonical DAV object. A destination
Space may contain a local materialized reference/facet that combines a bounded source
cache with destination-user-owned private fields. See
[cross-space-composition.md](cross-space-composition.md).

### Foreign source identity

Durable foreign identity is:

```text
source_account_fingerprint
source_space_id
source_object_id
source_kind
source_dav_uid              # optional correlation only
```

Do not use binding fingerprint, display name, email/title, or DAV UID alone to retarget
a reference. Routing provenance such as source domain may be retained separately, but a
domain/configuration change is not itself a foreign-identity change.

### Field ownership

Materialized schemas distinguish:

- **source-owned** bounded cached fields refreshed by Any-Cal;
- **destination-user-owned** body/notes/tags/local relations, never overwritten by refresh;
- **derived** availability/staleness state;
- **local overrides** such as an explicit private title.

Missing optional native metadata keeps the body-only representation usable. An
incompatible or ambiguous managed property fails before mutation; Any-Cal does not
coerce a private value into a source-owned field.

### Native reference metadata

The materialized record's JSON body remains the authority for foreign identity,
composition provenance, cached source fields, and private `user_fields`. The following
eight optional native **text** properties form one managed group:

| Stable property key | Source-owned value |
| --- | --- |
| `anycal_profile_fingerprint` | Principal/destination/source composition profile fingerprint |
| `anycal_foreign_identity` | Exact foreign identity fingerprint |
| `anycal_source_account_fingerprint` | Source upstream-account fingerprint |
| `anycal_source_space_id` | Source Space ID |
| `anycal_source_object_id` | Source object ID |
| `anycal_source_kind` | `contact`, `contact_group`, `task`, or `event` |
| `anycal_source_dav_uid` | Optional DAV UID correlation; an absent UID is written as empty text |
| `anycal_source_status` | `available`, `archived`, `deleted`, or `unavailable` |

Projection uses `discover_domain_capabilities(destination_domain)` and its existing
snapshot, qualified by domain, upstream-account/binding fingerprint, Space, and schema
profile. Display names never identify types or properties. Every managed key must exist
exactly once with the `text` format before the group is written. A missing optional key
disables the whole native group; a wrong format or duplicate key returns a bounded
schema error before a destination mutation. Existing managed values must also have
unambiguous text formats before Any-Cal replaces them.

Generic `materialize_references` can use the built-in `page` type without discovery or
custom metadata. Explicit typed operations, including `materialize_person_contexts`,
require a current snapshot containing the requested usable native type even when the
optional metadata group is absent. Discovery is read-only: these calls do not create
types or properties. Rediscover after restart, a failed discovery refresh, or upstream
schema changes. Binding qualification prevents reuse in a different context; it does
not detect a later remote schema edit without another discovery request.

Materialization results report `BodyOnly` or `Native` metadata projection. Discovery
diagnostics identify individual missing or incompatible keys. Missing optional metadata
does not make canonical body-only DAV storage unavailable.

The managed ownership set is fixed, including the optional UID. Native refresh sends
empty text when a previously stored UID disappears, instead of leaving its old value
through PATCH omission. Body-only refresh omits all unvalidated managed keys; it neither
clears nor treats an older remote native value as authoritative. Private properties,
including ordinary text notes, tags, and same-Space object relations, stay outside the
managed group and retain their values and formats. Known read-only system fields are
filtered by the shared HTTP update encoder.

After an ambiguous native write, readback must confirm the intended managed values and
formats as well as the durable record. Equal bodies alone cannot confirm a metadata-only
update; an unconfirmed write remains a timeout rather than a reported refresh success.

Scripted HTTP tests cover schema discovery, create, hydration, refresh, PATCH, and
reopen. They establish the request contract and ownership boundary; live Anytype
user-editing and two-principal interoperability remain the separate pilot gate.

### Person Context

A Person Context is a private local facet for a canonical Contact and is not exposed as
an additional CardDAV contact by default.

The implemented source-owned cache includes exact foreign identity, source display name,
bounded emails/phones/organization, DAV UID correlation, and source availability. Native
private text properties, tags, and same-Space relations are destination-owned. The
object body contains the durable materialized record; replacing it with free-text notes
is not supported.

The visible title is the native top-level `name`, separate from custom properties.
`person_context_title` in the durable record tracks either `Source { last_written }` or
`Local` ownership. New contexts derive a useful title from the bounded display name,
DAV UID, or `Person Context`. Source-owned titles follow renames only while the observed
native name exactly matches the last applied title. A local edit switches ownership
permanently to `Local`, including an empty or whitespace-only name, without copying that
private title into the source cache or managed metadata.

Older records without this marker preserve any observed native title as local. A missing
native name on an existing object is unknown: no title is written, and the previous
marker is retained. The optional marker extends version 1; older strict readers reject
newly enriched records. See the complete [title ownership policy](cross-space-composition.md#field-ownership).

Ambiguous create/update confirmation includes the intended native name, even with
body-only metadata projection. Fixtures cover initial names, source renames, ordinary
native title/note edits, local overrides, restart, and timeout readback. These sequential
checks do not establish concurrent-edit conflict resolution or live interoperability.

### Event Reference and Daily Plan

An Event Reference is a private local reference for a canonical VEVENT. Candidate
source cache includes title, start/end/all-day, timezone/floating summary,
location/status, exact foreign identity, and availability. Canonical VEVENT recurrence
and timezone semantics remain authoritative.

A Daily Plan is destination-owned, not a DAV calendar resource. Its deterministic
identity includes principal scope + destination domain + local date + schema/version.
It relates to same-Space Event Reference objects. Recurring occurrence identity must be
explicit and is finalized with VEVENT recurrence semantics.

## Contacts and Contact Groups (CardDAV)

`Contact` and `Contact Group` are separate Anytype Types.  Both are CardDAV
address-object resources; `KIND:individual`, `KIND:org`, and `KIND:group`
are stored in `dav_kind`/a visible Kind property.

| CardDAV/vCard concept | Anytype property | Notes |
| --- | --- | --- |
| `FN` | Object name | Required display name |
| `N`, `NICKNAME`, `SORT-STRING` | Name component properties | Generated when configured |
| `ORG`, `TITLE`, `ROLE` | Organisation, title, role | Native text properties |
| `EMAIL`, `TEL`, `ADR`, `IMPP`, `URL` | Visible properties plus `dav_fields` | Values/labels are repeated structured occurrences |
| `BDAY`, `ANNIVERSARY` | Date properties | Preserve original precision/parameters in `dav_fields` |
| `CATEGORIES` | multi-select Tags | Native array mapping |
| `NOTE` | body / Notes property | Mapping is configurable |
| `RELATED` | Object relation | Also retain URI/type in `dav_fields` |
| `PHOTO`, `LOGO`, `KEY` | File/media property | Preserve in `dav_fields` until file delivery is implemented |
| `MEMBER` | `Members` object-array on Contact Group | Links to Contact objects |

For predictable DAV resource paths, use `/carddav/<address-book>/<anytype-id>.vcf`.
The `dav_uid` remains the vCard UID and must not be inferred from that path.

**[RESEARCH]** The public Anytype API documents scalar `phone`, `email`,
`url`, and `text` values; it documents arrays for `objects`, `files`, and
`multi_select`, but not arbitrary structured lists.  Validate whether current
Anytype UI/API offers another native list representation before choosing the
hidden structured field as the definitive implementation.

**[RESEARCH]** Test Apple Contacts, Android/DAVx5, and Thunderbird with
groups, `RELATED`, labelled phone/email values, photos, vCard 3.0 versus 4.0,
and unknown `X-` properties.  Determine the smallest interoperable profile.

## Tasks (CalDAV VTODO)

`Task` is an Anytype Type.  A `DAV Task List` object is exposed as one CalDAV
calendar collection.  Each Task has exactly one `DAV List` object relation,
which determines its collection.  It can additionally link to zero or more
Projects without changing its DAV collection.

| VTODO property | Anytype property | Notes |
| --- | --- | --- |
| `SUMMARY` | Object name | Required task title |
| `DESCRIPTION` | body | Markdown conversion rules required |
| `STATUS` | Status select | Map standard values exactly |
| `DTSTART`, `DUE`, `COMPLETED` | Start, Due, Completed dates | Preserve timezone/floating semantics |
| `PERCENT-COMPLETE` | Progress number | 0–100 |
| `PRIORITY` | Priority number/select | iCalendar uses 0–9 |
| `CATEGORIES` | Tags multi-select | |
| `RELATED-TO` | Parent Task / related-object relation | Preserve relation type in `dav_fields` |
| `LOCATION`, `URL` | properties | |
| `RRULE`, `RDATE`, `EXDATE`, `VALARM`, `ATTENDEE` | `dav_fields` initially | Add visible mappings only when supported |

Projects remain normal Anytype objects with a `Tasks` object-array relation;
they may embed/query linked tasks.  A project becomes a DAV List only when the
user explicitly marks it as such.  This prevents ordinary project relations
from creating or moving CalDAV collections.

Task paths use `/caldav/<task-list-id>/<anytype-id>.ics`.  The server exposes
the collection as VTODO-capable and does not claim VEVENT support there.

**[RESEARCH]** Test Tasks.org's exact VTODO output, updates, deletions,
subtasks, tags, recurrence and alarms.  In particular, determine its use of
`RELATED-TO` and any vendor `X-` fields before finalising mappings.

**[RESEARCH]** Decide how to handle a task linked to several projects:
single primary DAV List is the safe default, but a user may expect multiple
DAV list appearances.

## Calendar events (CalDAV VEVENT)

`Calendar Event` is a separate Anytype Type.  A `DAV Calendar` object maps to
one CalDAV calendar collection.

| VEVENT property | Anytype property | Notes |
| --- | --- | --- |
| `SUMMARY` | Object name | |
| `DTSTART`, `DTEND`, `DURATION` | Start/end dates | Core MVP fields |
| `DESCRIPTION` | body | |
| `LOCATION`, `URL` | properties | |
| `STATUS`, `TRANSP`, `CLASS` | select properties | |
| `CATEGORIES` | tags multi-select | |
| `RRULE`, `RDATE`, `EXDATE`, exceptions | `dav_fields` initially | Recurrence model is deferred |
| `ATTENDEE`, `ORGANIZER`, `VALARM`, attachments | `dav_fields` initially | No scheduling claim in MVP |

**[RESEARCH]** Date/time semantics, time zones, recurrence expansion and
exceptions are complicated enough that VEVENT date-range query support must
be proven with tests before being advertised.  Calendar scheduling/free-busy
is explicitly out of the initial scope.
