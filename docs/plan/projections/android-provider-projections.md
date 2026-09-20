# Android provider projection contracts

**Status:** design only; no provider implementation.

This document turns the Android provider research into a narrow Rust/Kotlin
contract. Anytype/DAV remains canonical. Android rows are projections and can
normalize or omit data. Unknown and unsupported fields remain in the canonical
opaque property list.

## Canonical envelope

Rust should expose a versioned, JSON-like model to Kotlin rather than Android
`Cursor`, Binder, or framework objects:

```text
ProjectionEnvelope {
  schema_version: 1,
  kind: contact | event | task,
  canonical_id: Anytype object ID,
  dav_uid: optional stable vCard/iCalendar UID,
  source: {
    adapter: android_contacts | android_calendar | tasks_org,
    account_name: string,
    account_type: string,
    source_id: string,             # SOURCE_ID or provider _SYNC_ID
    provider_row_id: string,       # operational only
  },
  fields: typed canonical fields,
  opaque: [PropertyOccurrence],    # name, value, parameters, order
  projection_hash: string,
  canonical_revision: string,
  tombstone: optional {revision, deleted_at},
}
```

`source_id` is deterministic: use `android/<account-type>/<account-name>/<canonical-id>`
when the provider is Any-Cal-owned. Imported records retain the observed
provider source ID and a separate canonical ID. Never derive identity from a
display name, aggregate contact ID, or calendar/event row ID.

`PropertyOccurrence` is a lossless-as-observed structure:

```text
{ name: string, value: string/bytes, parameters: map<string, list<string>>, order: integer }
```

The canonical model has explicit fields only where a provider projection has a
stable contract. Every update merges explicit fields with opaque occurrences;
it must not drop unknown properties.

## Cardinality and conflict rules

| Field class | Canonical cardinality | Provider projection | Conflict rule |
| --- | --- | --- | --- |
| name/title, UID, status | single | one provider field | Last-write policy only after revision/hash comparison. |
| phone, email, URL, address | list of labelled values | one `Data` row per occurrence | Match by source occurrence key, not position. |
| group/project/list relation | set of stable IDs | membership/list foreign keys | Add/remove set members independently. |
| recurrence/reminders/attendees | ordered or keyed list | provider child rows/fields | Compare semantic normalized form; retain original opaque form. |
| vendor/unknown fields | list of occurrences | usually unsupported | Canonical-only unless capability probe proves support. |
| photo/attachments | binary object list | provider-specific | Hash content; unsupported provider writes remain canonical-only. |

For each source occurrence, derive a stable key from `(property name,
normalized parameters, value hash, occurrence discriminator)`. Preserve an
explicit `source_occurrence_id` when the provider exposes one. Never identify a
repeated phone/email by array position.

Provider-to-Anytype changes are accepted only when the observed provider hash
differs from the last projected hash. Anytype-to-provider updates carry the
canonical revision and expected provider hash. A mismatch produces a conflict
record; it does not silently overwrite the external edit. Tombstones suppress
resurrection until the canonical revision or explicit user restore supersedes
them.

## ContactsContract contract

### Rust fields

```text
ContactEnvelope {
  display_name: string,
  name: {family, given, middle, prefix, suffix, phonetic...},
  phones: list<LabelledValue>,
  emails: list<LabelledValue>,
  addresses: list<LabelledAddress>,
  urls: list<LabelledValue>,
  organizations: list<Organization>,
  notes: list<string>,
  birthdays_anniversaries: list<PartialDate>,
  relations: list<LabelledValue>,
  groups: set<CanonicalGroupId>,
  photos: list<BinaryRef>,
  opaque: list<PropertyOccurrence>,
}
```

Kotlin maps these to `RawContacts`, `Data`, `Groups`, and group-membership
rows. One Any-Cal contact owns one raw-contact row. Each repeated value is a
separate `Data` row. `RawContacts.SOURCE_ID` carries the deterministic source
identity; `Contacts._ID` is only an aggregate/view ID. Group creation and
membership are separate operations and must be idempotent.

Unsupported or uncertain mappings include arbitrary vCard parameters, custom
X-properties, multiple notes, exact date precision, photo variants, and
nonstandard relationship labels. Keep these in `opaque` and mark the field
status as `preserved`, `normalized`, or `unsupported` in the apply result.

### Contact invariants

1. Replaying the same envelope produces no new raw-contact or data rows.
2. Reordering repeated values does not create updates.
3. A group membership removal does not delete the contact or group.
4. Aggregate contact ID changes do not change `SOURCE_ID` identity.
5. Provider-normalized values are compared semantically and the original
   canonical occurrence remains available for DAV output.

## CalendarContract contract

### Rust fields

```text
EventEnvelope {
  calendar_canonical_id: string,
  summary: string,
  description: optional string,
  location: optional string,
  start: DateTimeValue,
  end: optional DateTimeValue,
  duration: optional string,
  all_day: bool,
  timezone: optional string,
  status/transparency/class: optional enums,
  recurrence: {
    rrule: optional string,
    rdate: list<DateTimeValue>,
    exdate: list<DateTimeValue>,
    exceptions: list<EventEnvelope>,
  },
  attendees: list<Attendee>,
  organizer: optional Attendee,
  reminders: list<Reminder>,
  opaque: list<PropertyOccurrence>,
}
```

Kotlin maps calendars to account-owned `Calendars` rows and events to
`Events`. Use provider sync columns (`_SYNC_ID`, and dirty/deleted state where
exposed) only as provider-facing identity/state. Retain `dav_uid` separately.
Map attendees and reminders to their child rows. Treat `Instances` as a
computed query, never as canonical event objects.

Recurrence is semantic data: preserve the original RRULE/RDATE/EXDATE and
compare a normalized recurrence model after provider expansion or rewrite.
Floating times, all-day dates, DST boundaries, attendee response status,
multiple reminders, and `ExtendedProperties` require capability/device tests.

### Calendar invariants

1. Replaying an event does not duplicate calendars, attendees, or reminders.
2. Event identity remains stable when the provider row ID changes.
3. Editing an occurrence does not silently rewrite the canonical recurrence
   rule without an explicit exception operation.
4. Provider instance expansion never creates independent canonical events.
5. Unsupported iCalendar properties remain in `opaque` and are not erased by
   an otherwise ordinary edit.

## Minimal Tasks.org adapter

Tasks.org is not a platform contract. Define a capability-gated interface:

```text
TasksOrgCapability {
  package_name: string,
  version_code: integer,
  authority: string,
  api_version: string,
  permissions: {read: bool, write: bool},
  operations: set<list, task, tag, reminder, observer>,
  columns: set<string>,
}

TasksOrgAdapter {
  probe() -> TasksOrgCapability | unsupported,
  list_existing_lists() -> list<ListRef>,
  pull_changes(checkpoint) -> changes,
  apply_task(TaskEnvelope, existing_list_id) -> ApplyResult,
  delete_task(source_id) -> ApplyResult,
}
```

The adapter is enabled only for an allow-listed package/version and a
successful disposable CRUD probe. It must refuse an absent authority,
permission mismatch, missing column, or version mismatch without affecting
ContactsContract/CalendarContract. Task creation must require an existing
list reference; it must never create a list as a side effect.

Minimum task mapping: title, notes, due/start, completion, list, tags,
parent, recurrence, and reminders. Every field result is classified as
preserved, normalized, or unsupported. Tasks.org 15.10 must be tested
separately from releases documenting `org.tasks.api`; do not infer support from
the newer API documentation.

## Kotlin operation contract

```text
probe(adapter, account) -> capability fingerprint
pull(scope, checkpoint) -> envelopes + tombstones + next checkpoint
apply(batch, expected_hashes) -> per-item IDs, hashes, field statuses, conflicts
observe(scope) -> debounced change signal only
```

Kotlin owns `ContentResolver`, permissions, account metadata, provider IDs,
batch limits, and observers. Rust owns canonical merge/conflict policy and
checkpoint advancement. A failed operation must identify whether it is
retryable, unsupported, permission-related, or a conflict.

## Focused acceptance tests

### Contacts

- Project two labelled phones, two emails, two addresses, name components,
  organization, group membership, photo, and an X-property; replay twice and
  assert stable row counts and identity.
- Edit one phone in the system Contacts UI; pull once, preserve the edit, and
  assert that the observer callback does not cause a write loop.
- Remove a group membership and then delete the contact; verify independent
  tombstones and no resurrection after restart.
- Revoke `WRITE_CONTACTS`, remove/re-add the Any-Cal account, and verify that
  no unrelated account rows are modified.

### Calendar

- Project UTC, local, floating, all-day, DST-boundary, recurring, attendee,
  and multiple-reminder events; replay and compare semantic fields.
- Edit an attendee, reminder, and event time in the system Calendar UI; verify
  conflict/hash handling and preservation of canonical recurrence/opaque data.
- Delete an event and its calendar separately; verify tombstones and account
  ownership after process restart.
- Recreate provider rows with the same source IDs and verify reconciliation
  rather than duplication.

### Tasks.org

- Probe absent provider, denied custom permissions, unsupported version, and
  missing columns; each must return `unsupported` without blocking other
  adapters.
- With one existing synced list, create/update/complete/delete a synthetic
  task and assert no list creation or unrelated collection mutation.
- Round-trip every supported task field and record normalization/unsupported
  results; verify observer restart and checkpoint recovery.

## Evidence and unresolved risks

Primary contracts: [ContactsContract](https://developer.android.com/reference/android/provider/ContactsContract),
[CalendarContract](https://developer.android.com/reference/android/provider/CalendarContract),
[sync adapter](https://developer.android.com/training/sync-adapters/creating-sync-adapter),
and [Tasks.org provider documentation](https://raw.githubusercontent.com/tasks/tasks/main/CONTENT_PROVIDER.md).

Still requiring device evidence: provider tombstone retention, OEM label and
parameter normalization, recurrence/attendee/reminder rewrites, account
removal semantics, observer timing, and Tasks.org authority/schema behavior.
