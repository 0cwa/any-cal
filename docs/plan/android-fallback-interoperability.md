# Android direct-provider fallback interoperability

**Status:** validation design; no live EteSync/DAVx⁵ account or provider claim.

This lane defines how an Any-Cal-owned Android provider projection coexists
with DAVx⁵, EteSync, and ordinary CalDAV/CardDAV clients. The desktop
Anytype-backed DAV service remains the interoperability baseline. This
document does not change the Android implementation or canonical ledger.

## Core rule: one writer per dataset

Ownership is selected independently for contacts, calendars, and tasks. A
dataset means one logical address book, calendar collection, or task list—not
the whole Android device.

| Mode | Android provider writer | Any-Cal behavior | Safe coexistence |
| --- | --- | --- | --- |
| Any-Cal direct | Any-Cal account adapter | canonical Anytype ↔ provider projection | DAVx⁵/EteSync must not write the same collection/account. |
| DAV relay | DAVx⁵ or EteSync adapter | Anytype DAV endpoint is the server; no direct provider writes | Any-Cal observes server-side DAV changes, not the provider rows. |
| Import-only | Existing external account | one-way provider read into Anytype | No write-back and no deletion of external rows. |

Never enable Any-Cal direct writes and DAVx⁵/EteSync writes for the same
provider rows. If a user needs both, use separate server collections and
Android accounts with explicit ownership labels. “The records look identical”
is not sufficient isolation: source IDs, tombstones, and loop metadata must
also be separate.

Tasks require an additional gate: Android has no standard task provider.
Tasks.org is an optional app-specific adapter, while DAVx⁵/OpenTasks/EteSync
task paths are separate adapters and must not be inferred to be interchangeable.

## Identity and loop prevention

The canonical identity is `(collection canonical ID, DAV UID)` for DAV objects.
Provider IDs are projections:

```text
Any-Cal-owned Contacts: RawContacts.SOURCE_ID = any-cal source ID
Any-Cal-owned Calendar: Calendars/Events _SYNC_ID = any-cal source ID
External provider:     observed provider source ID + collection/account scope
```

Persist the following for every projected/imported record:

```text
canonical_id, dav_uid, collection_id
source_adapter, source_account, provider_row_id, provider_source_id
last_observed_hash, last_projected_hash, canonical_revision
ownership_mode, origin, tombstone_revision
```

Loop suppression is hash- and ownership-based:

1. A write records operation ID, source identity, expected hash, and resulting
   hash.
2. A provider callback matching the resulting hash is an echo and is ignored.
3. A differing hash is an external edit and enters merge/conflict handling.
4. A record with an external source ID is never adopted as Any-Cal-owned just
   because its DAV UID matches.
5. A server-side DAV change is applied only to the owner adapter for that
   collection.

Do not use title, start time, email, or display name as a primary duplicate
key. They are only fallback evidence when source identity is unavailable and
must produce a reviewable ambiguity rather than an automatic merge.

## Import and export boundaries

### Any-Cal direct mode

Any-Cal creates one Android account and account-owned provider rows. The Rust
canonical envelope retains repeated values, original parameters, vendor
properties, recurrence, and unsupported fields. Kotlin only projects fields
the capability probe says the provider can write.

Export to DAV is canonical Anytype output, not a re-export of normalized
Android rows. Provider edits can be imported only after the identity, hash,
deletion, and conflict tests pass.

### DAV relay mode

DAVx⁵ or EteSync owns the Android provider rows. Any-Cal communicates with the
DAV service and must not query, rewrite, or delete those provider rows as a
second sync adapter. The server-side DAV UID/ETag/resource path is the source
of truth for reconciliation.

If Anytype and a DAV client are both consumers of the same server collection,
they are ordinary DAV clients. The server's ETag/conditional-write rules and
UID/collection identity prevent duplicate resource creation; Android provider
row IDs remain outside that protocol identity.

### Import-only mode

Import records with `ownership_mode=import-only` and preserve the observed
provider account/source ID. Never send edits or deletions back to that account
until the user explicitly promotes the dataset to Any-Cal ownership. Promotion
requires a duplicate scan, an ownership confirmation, and a new projection
checkpoint.

## Account removal and recreation

Account lifecycle is not a delete shortcut:

1. On account removal, mark its projection `account-missing` and stop writes.
2. Retain canonical records and tombstones in Any-Cal.
3. Do not delete rows belonging to another account or adapter.
4. On re-addition, match by account type/name plus source ID; never recreate
   by display name alone.
5. If the provider recreated rows with new row IDs, repair operational links
   from `SOURCE_ID`/`_SYNC_ID` and compare hashes before updating.
6. If source IDs disappeared, stop and require a duplicate/ownership review.

Provider behavior after account removal is OEM/provider-specific. The policy
above is the Any-Cal safety policy, not a claim that Android retains rows.

## Duplicate and collision handling

Use this ordered matching strategy:

1. exact `(adapter, account, collection, provider_source_id)`;
2. exact `(collection, DAV UID)` when the source is a DAV resource;
3. explicit canonical ID stored in provider sync metadata;
4. normalized fingerprint plus a human-reviewable collision result.

The fingerprint may include normalized title/name, dates, and selected values,
but must never silently merge two candidates. A collision is a durable result
with both source identities and candidate hashes. A duplicate imported from
DAVx⁵ and Any-Cal direct mode should remain two records until ownership is
resolved; automatic deletion is unsafe.

## Existing executable evidence

The sanitized profiles under `fixtures/protocol/android/` cover DAVx⁵-shaped
discovery/contact traffic and Tasks.org-shaped VTODO CRUD. The runner is
`crates/dav-server/tests/android_profile.rs`; it runs each profile twice and
compares normalized response traces and final resource snapshots. It checks:

- repeated labelled contact values and opaque X-fields;
- VTODO UTC/floating/TZID values, create/edit/complete, and deletion/archive;
- stale conditional writes and per-resource 404s;
- injected read failures followed by deterministic replay;
- stable response headers and ETags.

Run:

```text
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test android_profile --offline
```

Observed result on 2026-09-19:

```text
test result: ok. 2 passed; 0 failed; 0 ignored
```

This proves only the in-process DAV protocol profile. It does not prove that
DAVx⁵, EteSync, Tasks.org, ContactsContract, or CalendarContract executed the
same requests, nor does it prove provider ownership or background scheduling.

## Focused fallback acceptance matrix

### Mutual exclusion

- Enable Any-Cal direct mode for a synthetic calendar; verify DAVx⁵/EteSync
  ownership of that exact collection is rejected or clearly disabled.
- Enable DAV relay mode; verify Any-Cal does not write Android provider rows.
- Enable import-only mode; edit/delete a source row and verify no write-back.
- Attempt to configure two writers for one `(account, collection)` and require
  a blocking conflict, not last-writer-wins activation.

### Identity and duplicates

- Import the same synthetic DAV object through DAV relay and direct provider
  discovery; verify source namespaces remain distinct until explicit merge.
- Recreate provider rows with new row IDs but the same source ID; verify repair
  rather than duplicate creation.
- Remove source ID metadata or produce two candidates; verify reviewable
  ambiguity and no automatic merge/delete.

### Deletion and lifecycle

- Delete in Anytype, DAVx⁵, and the Android provider separately under the
  selected ownership mode; verify only the owner propagates tombstones.
- Remove and re-add the Any-Cal account; verify canonical records survive and
  unrelated accounts remain untouched.
- Stop/restart the process after a failed write; verify checkpoint replay is
  idempotent and cannot resurrect a tombstone.

### Fallback isolation

- Remove or deny the optional Tasks.org provider; Contacts/Calendar direct
  projections and DAV relay remain available.
- Disable direct CalendarContract capability; DAV calendar access remains
  available through the desktop endpoint.
- Make a DAV server collection temporarily unavailable; provider data remains
  tagged with its last owner/checkpoint and is not adopted by another adapter.

## Deferred live validation

The following require a disposable API-35+ emulator or explicitly supplied
device, pinned DAVx⁵/EteSync/Tasks.org builds, and synthetic credentials:

- actual Android account ownership and provider row creation/removal;
- DAVx⁵ contact/group/photo normalization and calendar recurrence behavior;
- EteSync native app/provider exposure and account lifecycle;
- Tasks.org 15.10 versus the release documenting `org.tasks.api`;
- Android Doze/OEM scheduling, ContentObserver timing, and process restart;
- real CalDAV/CardDAV ETag/UID behavior across two independent clients.

No personal Android account or live EteSync credentials should be used for
these tests. Passing the offline profiles must not be promoted to live-client
compatibility evidence.

## Integration disposition

The safe implementation default is **import-only** for existing Android
accounts and **one writer per explicitly owned dataset** for Any-Cal direct
projection. DAVx⁵ remains the generic Android fallback. EteSync should be
treated as a separate server/client path until a disposable device test proves
its provider exposure and ownership semantics. Tasks.org remains capability-
and-version-gated, never a platform-task dependency.
