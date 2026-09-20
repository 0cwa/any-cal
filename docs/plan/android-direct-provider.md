# Android direct-provider architecture

**Status:** proposed Android extension; the desktop DAV service remains the
primary cross-platform interoperability surface.

## Decision

The Android companion should synchronize Anytype directly into Android's
system providers. DAVx⁵ is not required for the normal Contacts or Calendar
path.

```text
Anytype <-> Rust sync core <-> Kotlin Android adapters
                              |- ContactsContract
                              |- CalendarContract
                              `- optional Tasks.org provider
```

The existing DAV server remains useful for desktop clients, generic Android
clients, Tasks.org installations without a supported provider API, and users
who explicitly want DAVx⁵.

The Rust side should own Anytype transport, canonical DAV-shaped documents,
identities, checkpoints, tombstones, retries, and conflicts. The Kotlin side
should own Android permissions, account/provider APIs, background scheduling,
and provider-specific row IDs. Expose a small FFI-safe sync request/result
interface (UniFFI or a narrow JNI ABI); do not expose Rust traits directly.

## Evidence and assumptions

Verified facts:

- `ContactsContract` is Android's contacts provider.
- `CalendarContract` is Android's calendar/event provider and supports
  account-owned sync-adapter rows and `_SYNC_ID` values.
- Android sync adapters require an account type/authenticator boundary.
- DAVx⁵ writes synchronized data into Android providers; it is not a general
  local DAV database.
- Tasks.org documents a provider at `content://org.tasks.api/v0` with task,
  list, tag, reminder, CRUD, and change-observer APIs.
- Tasks.org labels that provider API `v0` unstable.
- Android has no platform-standard task provider. The OpenTasks provider is a
  separate project/module and its standalone provider repository is deprecated;
  it is not evidence of Tasks.org compatibility.

Tested versions, not compatibility guarantees:

- Android API 35 emulator.
- DAVx⁵ 4.5.19-ose.
- Tasks.org 15.10.

The tested Tasks.org 15.10 build predates the provider API documented in the
current 15.12-era source/changelog. Runtime authority and version detection is
therefore mandatory. Direct Tasks.org integration remains optional until both
15.10 and 15.12 tests establish the supported boundary; do not assume that
the newer provider exists in 15.10.

## Provider mappings

### ContactsContract

Create one Any-Cal Android account and one `RawContacts` row per Anytype
contact. Store the stable Anytype object/DAV identity in `RawContacts.SOURCE_ID`.
Project names, phones, emails, organization, addresses, URLs, notes, and
groups into `ContactsContract.Data` MIME types. Use Groups and group-membership
rows where the source has explicit group identity.

Android rows are a projection, not the canonical record. Preserve repeated
values, parameters, vendor fields, photos, and unsupported fields in Anytype's
canonical envelope. Provider normalization may be lossy.

### CalendarContract

Create account-owned calendars for selected Anytype collections. Store the
stable source identity in calendar/event `_SYNC_ID` values. Project event
title, description, location, start/end, all-day state, timezone, attendees,
reminders, recurrence, and supported extended properties.

CalendarContract may normalize recurrence and timezone data. Anytype retains
the canonical representation and unsupported iCalendar fields.

### Tasks.org (optional)

When the authority `org.tasks.api` is present and its installed version is in
the supported matrix, use its provider rather than DAVx⁵:

| Any-Cal task data | Tasks.org provider |
| --- | --- |
| title | `title` |
| notes/body | `notes` |
| due date/all-day | `due_date`, `due_all_day` |
| start date/all-day | `start_date`, `start_all_day` |
| completion | `completed_at` |
| recurrence | `recurrence`, `repeat_from` |
| project/list | `list_id` |
| tags | `task_tags` / `tag_ids` |
| parent task | `parent_id` |
| reminders | `/v0/reminders` |

Request `org.tasks.permission.READ_TASKS` and
`org.tasks.permission.WRITE_TASKS` only when the user enables this adapter.
First resolve the `org.tasks.api` authority, read the installed package/version,
and probe the exact endpoint/columns needed by the selected mapping. Use
provider capability/version detection, paging, background-thread Binder calls,
and a `ContentObserver` on the collection URI. The provider API is unstable,
so a missing authority, denied permission, missing endpoint, or changed column
must disable only this adapter and leave Contacts/Calendar synchronization
working.

Do not treat OpenTasks or an arbitrary task application as a generic task
target without a separate adapter and compatibility evidence. The OpenTasks
provider and Tasks.org provider are separate APIs.

## Accounts, permissions, and identity

Use an Android account type owned by Any-Cal and an account name that identifies
the configured Anytype space without embedding the API token. Register the
required authenticator/sync metadata. Request contacts/calendar permissions
only for enabled projections; request Tasks.org's custom permissions only when
Tasks.org integration is enabled.

Never use Android row IDs as durable identity. Persist:

```text
Anytype object ID
DAV UID
Android account/type
Android provider row ID
last projected hash
last provider modification observation
last Anytype revision
origin/direction
```

Writes made by Any-Cal must be distinguishable from user edits. Account
removal, provider recreation, permission loss, and missing rows are explicit
state transitions, not implicit creates.

## Sync and background policy

The first milestone is Anytype-to-provider projection only. Add provider-to-
Anytype writes after identity, deletion, and conflict tests pass.

Use WorkManager for bounded sync work and provider observers for prompt local
change signals. A retry must be idempotent and resume from the Rust checkpoint.
Do not promise immediate synchronization under Android Doze or OEM background
restrictions.

## Milestones

1. Capability probe: detect permissions, account/provider availability, and
   Tasks.org authority/version without writes.
2. One-way ContactsContract projection.
3. One-way CalendarContract projection.
4. Optional one-way Tasks.org projection with a version matrix.
5. Bidirectional contacts and calendar edits, including tombstones/conflicts.
6. Bidirectional Tasks.org edits and reminders, only after the 15.10/15.12
   capability tests pass.
7. DAVx⁵ fallback and generic-client compatibility.

The canonical ledger splits these into dependent units:

1. `android-direct-provider-foundation`: capability probe, Rust/Kotlin FFI
   seam, account/authenticator, permissions, and background constraints.
2. `android-direct-provider-projections`: ContactsContract, CalendarContract,
   and optional version-gated Tasks.org projections. Depends on foundation.
3. `android-direct-provider-validation`: conflict/loop prevention, emulator/CI
   validation, packaging/release, and fallback interoperability. Depends on
   projection evidence.

The separate Anytype live CRUD unit remains independently blocked by its
isolated safe-change/VM requirements; Android provider work does not authorize
or depend on live Anytype writes.

## Acceptance gates and stop rules

Do not begin provider writes until the no-write capability probe records the
API level, provider authorities, permissions, Tasks.org package/version, and
supported columns. Do not begin bidirectional sync until one-way projection is
idempotent and identity/deletion tests pass. Do not claim Tasks.org support
until both the tested 15.10 absence/behavior and the 15.12-era provider path
are explicitly classified.

Stop or defer the affected adapter when permissions are denied, a provider is
absent, an account is removed, an identity is ambiguous, a required field has
changed, or background execution cannot meet the retry contract. Contacts and
Calendar synchronization must continue when the optional Tasks.org adapter is
unavailable.

Defer direct provider work and retain DAVx⁵ fallback if Android provider
normalization makes required fields irrecoverably lossy, if the FFI seam cannot
share durable sync semantics safely, or if emulator/CI evidence cannot prove
account-removal and restart recovery. Do not substitute a local Android DAV
server merely to avoid a missing task provider.

## Research questions

- Which Android API levels and OEM behaviors preserve account-owned provider
  rows through restart, backup/restore, and account removal?
- Which ContactsContract MIME types and group operations preserve the required
  repeated labelled values and relationships?
- Which CalendarContract recurrence, timezone, attendee, and reminder fields
  need canonical-envelope retention or explicit loss markers?
- Exactly which Tasks.org versions expose `org.tasks.api/v0`, and which columns
  and permissions are stable across 15.10 and 15.12?
- Is WorkManager plus provider observers sufficiently reliable, or is an
  account sync adapter required for the supported Android range?
- Which FFI packaging approach gives reproducible ABI builds for Android
  architectures without exposing Anytype credentials to the UI layer?

## Risks and open research

- Contact normalization can lose vCard parameters, X-fields, photos, and
  group relationships.
- Calendar providers can normalize recurrence and timezone values.
- Tasks.org provider columns and semantics may change between releases.
- Account removal can delete provider rows.
- OEM scheduling restrictions can delay work.
- Provider permissions can be revoked independently of Anytype credentials.
- Bidirectional writes can loop without origin and hash tracking.

## Test matrix

Deterministic Rust tests must cover projection idempotence, repeated contact
values, labels, groups, unsupported fields, event recurrence/timezones,
Tasks.org list/tag/parent/reminder mappings, stable identity, archive/delete,
permission/provider absence, version-gated unsupported behavior, retries,
conflicts, and loop prevention.

Disposable API-35 emulator tests must cover permission grant/revocation,
system-app edits, account removal/re-addition, process restart, network loss,
Doze/background recovery, and provider row recreation. Run Tasks.org tests for
15.10 (where the provider may be absent) and 15.12 (where the provider is
documented) before claiming version support. A failed or absent Tasks.org
probe must not fail ContactsContract or CalendarContract synchronization.
Run DAVx⁵ separately as fallback evidence; direct Contacts/Calendar support
must not depend on DAVx⁵ being installed.

## Unsupported claims

This plan does not claim that all Android task apps expose a common provider,
that provider projections preserve byte-identical vCard/iCalendar data, or
that WorkManager guarantees a sync interval. It does not claim Tasks.org
provider compatibility beyond tested versions, and it does not replace the
desktop DAV interoperability path.

References: [Android ContactsContract](https://developer.android.com/reference/android/provider/ContactsContract), [Android CalendarContract](https://developer.android.com/reference/android/provider/CalendarContract), [Android sync adapter/authenticator](https://developer.android.com/training/sync-adapters/creating-authenticator), [Tasks.org provider API](https://raw.githubusercontent.com/tasks/tasks/main/CONTENT_PROVIDER.md).
