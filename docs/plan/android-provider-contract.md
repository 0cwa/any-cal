# Android provider contract and acceptance tests

**Status:** proposed companion-app contract; research and test plan only.

This document defines the boundary for an optional Android companion that
projects Anytype records into Android providers. It does not introduce an
Android-wide task API: Android has `ContactsContract` and `CalendarContract`,
but no platform task/VTODO provider. Tasks.org support is therefore an
explicit, version-gated adapter.

## Evidence boundary

The Android provider package documents contacts and calendar contracts, but no
`TaskContract` or VTODO table: [android.provider API](https://developer.android.com/reference/android/provider/package-summary).
The sync-adapter framework associates an account type with an arbitrary content
provider authority; it does not define the provider's tables or columns:
[creating a sync adapter](https://developer.android.com/training/sync-adapters/creating-sync-adapter).
The canonical records remain Anytype objects plus the DAV-shaped envelope.
Provider rows are projections and may normalize or discard information.

## Ownership and authority

Kotlin/Android owns:

- `AccountManager`, the Any-Cal account type, authenticator, sync-adapter
  metadata, and user-visible account settings;
- runtime permission requests and `ContentResolver`/`ContentProviderClient`
  calls;
- provider row IDs, paging, batches, `ContentObserver` registration, and
  Android scheduling.

Rust owns:

- Anytype transport and canonical object/DAV identity;
- field normalization, projection hashes, checkpoints, tombstones, and
  conflict decisions;
- the provider-neutral record model and capability policy.

The account name must identify the configured Anytype space without embedding
an API token. Every projection must be scoped to the Any-Cal account's
`ACCOUNT_NAME` and `ACCOUNT_TYPE`; never claim ownership from a display name
alone. Account removal or re-addition is a state transition requiring an
explicit reconciliation policy, not an implicit mass create/delete.

## Stable identity and tombstones

Persist an identity tuple for every projected record:

```text
anytype_object_id
dav_uid                         # canonical DAV identity, when available
android_account_name/type
provider_row_id                 # operational only; never durable identity
source_id_or_sync_id            # provider-side stable source identity
last_projected_hash
last_observed_provider_hash
last_anytype_revision
origin                           # anytype | provider | merged
deleted_at / tombstone_revision
```

Contacts:

- Create one account-owned `RawContacts` row per Anytype contact.
- Put the stable Anytype/DAV identity in `RawContacts.SOURCE_ID`.
- Put account name/type on the raw contact and use the returned raw-contact
  ID only for joins to `Data` rows, group membership, and photo rows.
- Do not use the aggregate `Contacts._ID` as source identity; aggregation can
  merge or split raw contacts.
- Preserve deleted source identities in Rust. A missing provider row is not
  automatically a create: distinguish provider deletion, account removal,
  permission loss, and provider database recreation.

Calendar:

- Create account-owned `Calendars` only for explicitly selected Anytype
  calendar collections.
- Use CalendarContract sync columns (`_SYNC_ID`, dirty/deleted state where
  exposed) for calendar and event source identity, and retain the canonical
  DAV UID separately.
- Store the provider calendar ID/event ID as operational foreign keys only.
- Treat provider tombstones and `DELETED`/dirty flags as signals to reconcile;
  exact retention and visibility vary by provider and require device tests.

The first implementation should use a durable local identity table in the
companion database. Provider sync columns are useful projections, not a
replacement for that table.

## ContactsContract projection

Use the documented `ContactsContract.RawContacts`, `Data`, `Groups`, and
`CommonDataKinds` contracts ([reference](https://developer.android.com/reference/android/provider/ContactsContract)).

| Canonical data | Provider projection | Contract notes |
| --- | --- | --- |
| name components | `StructuredName` | Provider may derive display name and normalize phonetics. |
| phones | `Phone` | Multiple rows; labels/type constants are provider-facing. |
| emails | `Email` | Multiple rows; preserve canonical parameter label separately. |
| organization/title/department | `Organization` | Multiple organizations may not round-trip identically. |
| postal addresses | `StructuredPostal` | Multiple rows; formatting and type labels may normalize. |
| URLs | `Website` | Multiple rows; vendor URI parameters need canonical storage. |
| note | `Note` | Usually one provider row; additional notes remain canonical-only. |
| birthday/anniversary | `Event` | Date precision and timezone behavior require testing. |
| relationships | `Relation` | Labels and custom relation types may be normalized. |
| groups | `Groups` + `GroupMembership` | Group identity and membership are separate rows. |
| photo | `Photo`/photo file | Binary and thumbnail behavior is provider-specific. |
| custom/vendor vCard fields | no guaranteed standard row | Preserve in Anytype/DAV envelope; optional provider-specific extension only. |

Repeated labelled values must be represented as separate provider `Data` rows,
not by overwriting a single row. Retain the original label and parameters in
the canonical record even when Android maps them to a type constant. Group
membership writes must be ordered after both the raw contact and group exist,
and group deletion must not silently delete the contact.

## CalendarContract projection

Use `Calendars`, `Events`, `Attendees`, `Reminders`, `ExtendedProperties`, and
the provider's documented sync columns ([reference](https://developer.android.com/reference/android/provider/CalendarContract)).

| Canonical data | Provider projection | Required handling |
| --- | --- | --- |
| collection | `Calendars` | Account-owned; stable source identity and capability check. |
| UID/title/description/location | `Events` | Keep DAV UID separately; provider IDs are not UIDs. |
| start/end/all-day/timezone | `Events` | Test floating times, DST, UTC, and all-day normalization. |
| recurrence | `Events.RRULE`/related fields | Preserve canonical RRULE; provider may rewrite it. |
| attendees/organizer | `Attendees` and event fields | Test response status, organizer, and attendee identity. |
| alarms | `Reminders` | Multiple reminders may be reordered, clamped, or unsupported. |
| vendor/extended fields | `ExtendedProperties` where supported | Never assume arbitrary iCalendar properties survive. |

Do not derive recurrence instances as independent source objects. The event
with its recurrence rule is canonical; `Instances` is a computed query view.
When a provider expands or rewrites recurrence, compare semantic recurrence,
not serialized bytes, and retain the original DAV representation in Anytype.

## Permissions, observers, and loop prevention

Request only enabled projection permissions: `READ_CONTACTS`/
`WRITE_CONTACTS` and `READ_CALENDAR`/`WRITE_CALENDAR`. A permission grant does
not prove that a provider supports the requested columns. Handle revocation as
a capability loss and stop writes before attempting repair.

Register `ContentObserver`s for the smallest relevant collection URIs (raw
contacts/data/groups and calendars/events/attendees/reminders). Observers are
change signals, not durable change logs: debounce them, query by account and
modified/dirty state where available, and reconcile from the identity table.
Use WorkManager or an equivalent bounded job for durable execution; do not
promise immediate synchronization under Doze or OEM restrictions.

Every write must carry enough local context to suppress its own observer
echo: operation ID, target source identity, pre/post projection hash, and
checkpoint. If the provider strips custom sync markers, use the local identity
table and post-write hash comparison. A provider edit that differs from the
last projected hash is a real external edit and enters conflict handling; a
same-hash callback is an echo/no-op.

## Optional Tasks.org adapter

There is no platform task provider. Tasks.org's documented
`content://org.tasks.api/v0` provider is an unstable, app-specific API in
newer releases ([source documentation](https://raw.githubusercontent.com/tasks/tasks/main/CONTENT_PROVIDER.md)).
Tasks.org 15.10 predates the documented API, so the adapter must be disabled
unless all of the following pass at runtime:

1. authority resolution and package/version allow-list;
2. read/write permission discovery and user grant;
3. probe of the exact task/list/tag/reminder columns and URI paths;
4. a disposable create/update/delete round-trip in a pre-existing list;
5. change-observer and restart behavior;
6. no collection/list creation when targeting an existing list.

If any probe fails, report `unsupported` for Tasks.org only. Contacts and
Calendar projections must continue independently. Do not treat OpenTasks or
another app's provider as a generic task target.

## Rust↔Kotlin boundary

Keep the ABI small and versioned. Kotlin sends capability and provider facts;
Rust returns deterministic projection operations.

```text
probe(request) ->
  provider kind/version, account identity, permission state,
  supported columns/operations, capability version

pull_changes(scope, checkpoint) ->
  provider rows/tombstones + source IDs + observed hashes + next checkpoint

apply(batch) ->
  create/update/delete operations keyed by canonical identity

apply_result ->
  per-operation provider IDs/source IDs, hashes, conflicts, unsupported fields,
  retryable/permanent errors, and provider checkpoint
```

Rust must never receive raw Android `Cursor` or Binder objects. Kotlin must
never implement Anytype conflict policy. The boundary must include an adapter
version and capability fingerprint so a provider upgrade or schema change
invalidates cached assumptions.

## Focused acceptance tests

Run deterministic projection tests in Rust and disposable API-35+ emulator
tests in Kotlin. Test data must be synthetic and include stable IDs, duplicate
labels, groups, recurrence, attendees, reminders, unsupported fields, and
intentional conflicts.

### Capability and safety gates

- Provider absent: probe returns `unsupported`; no writes; DAV/desktop path
  remains usable.
- Permission denied then revoked: reads/writes stop, state is recoverable, and
  no destructive cleanup occurs.
- Account absent, removed, or re-added: ownership is detected explicitly;
  re-add does not duplicate records.
- Provider/version mismatch: adapter reports unsupported with exact missing
  authority/column/version; other adapters remain active.
- Process restart/network loss/Doze: checkpoint resumes idempotently.

### Contacts

- Two phones, two emails, two labelled addresses, name components, note,
  organization, group membership, photo, and an unsupported X-property project
  without overwriting rows.
- Edit a projected row in the system Contacts UI; pull it once, preserve the
  edit, and do not generate an observer loop.
- Delete a contact in Anytype and in Contacts separately; verify tombstone
  propagation and no resurrection after restart.
- Remove the Any-Cal account and verify only account-owned rows are affected.
- Verify aggregate contact ID changes do not break raw-contact identity.

### Calendar

- Create an account-owned calendar and an event with UTC, local, floating,
  all-day, DST-boundary, recurrence, attendee, and multiple reminder data.
- Edit event/attendee/reminder in the system Calendar UI; compare semantic
  values and retain canonical RRULE/vendor fields.
- Delete event and calendar independently; verify tombstones and no accidental
  deletion of unrelated account calendars.
- Reconcile after provider row recreation and after permission revocation.

### Optional Tasks.org

- Run the capability matrix against 15.10 and a release that documents
  `org.tasks.api`; record authority, permission, columns, and exact behavior.
- Create a task in an existing synced list only; assert no list creation.
- Round-trip title, notes, due/completion, list, tags, parent, recurrence, and
  reminders, marking each field as preserved, normalized, or unsupported.
- Verify provider absence/version mismatch disables only this adapter.

## Unresolved research

- Exact provider tombstone retention and dirty-flag behavior across OEMs.
- Whether each ContactsContract data kind preserves arbitrary labels and
  parameters on the target Android builds.
- Calendar recurrence, attendee response, reminder, and extended-property
  normalization on stock and OEM providers.
- Account-removal semantics when a sync adapter owns rows but the user removes
  the account from Settings.
- Tasks.org authority/permission/schema behavior across supported releases.
- Whether provider observers deliver enough timing/URI detail to avoid a full
  scoped reconciliation after every callback.

These questions require emulator/device evidence before bidirectional support
is advertised. The desktop DAV endpoint remains the compatibility baseline.
