# Android account and sync lifecycle

**Status:** design lane for `android-direct-provider-foundation`.

This document defines ownership, lifecycle, scheduling, and recovery rules for
an Any-Cal Android companion. It does not claim that Android has a universal
task provider, and it does not authorize live Anytype or provider writes.

## Ownership model

Any-Cal owns one Android account per configured Anytype space:

```text
account type:  an Any-Cal domain-qualified constant
account name: an opaque, stable space label (not an API token)
data sets:    contacts, calendar, optional tasks.org projection
```

The account type must be unique to Any-Cal. The account name must remain stable
across process restarts and must not contain credentials. A display name is not
an identity. The durable identity is `(Anytype space ID, object ID)` in the
Rust-side database; Android row IDs are only operational references.

Contacts and Calendar use separate sync-adapter declarations for their
provider authorities (`com.android.contacts` and `com.android.calendar`). The
same account may be presented to the user as one Any-Cal account, but adapter
state, checkpoints, and failure handling remain independent per authority.

Primary Android references:

- [Contacts Provider](https://developer.android.com/identity/providers/contacts-provider)
- [Create a sync adapter](https://developer.android.com/training/sync-adapters/creating-sync-adapter)
- [Calendar provider](https://developer.android.com/identity/providers/calendar-provider)
- [AccountManager](https://developer.android.com/reference/android/accounts/AccountManager)
- [WorkManager constraints](https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work)

## Account and authenticator lifecycle

The companion contains an `AccountAuthenticatorService` and an
`AbstractThreadedSyncAdapter` service for each provider authority. The
authenticator is deliberately minimal: Any-Cal's setup UI obtains or accepts
the Anytype credential, validates the selected space, and adds the account.
The authenticator must not expose the token through account names, labels,
logs, intents, or sync-adapter extras.

Lifecycle states are explicit:

```text
absent
  -> configured (credential and space validated)
  -> enabled (account + selected projections)
  -> syncing / retryable-error / permission-blocked
  -> disabled (user paused projection)
  -> removed (account removed; tombstones retained locally for recovery window)
```

Account removal is not a normal sync deletion. On removal:

1. stop scheduling and cancel in-flight work;
2. mark the account generation closed in the local database;
3. stop all provider writes immediately;
4. retain identity/checkpoint/tombstone metadata for a bounded recovery period;
5. let Android remove account-owned provider rows according to provider
   semantics, then verify what remains without deleting rows by display name;
6. require explicit user action before re-adding and projecting again.

Re-adding the same space creates a new account generation but reuses canonical
object identity. It must reconcile existing rows by `SOURCE_ID`/`_SYNC_ID`
and the local mapping before creating anything. If ownership is ambiguous,
the adapter stops and asks for repair; it must not mass-create duplicates.

## Credentials and secrets

The token/API key belongs to the Rust sync service, not the UI or provider
rows. Store it only in Android Keystore-backed encrypted storage (or an
equivalent platform-protected secret store). Do not put it in:

- `Account` names or `userdata` bundles;
- provider `SYNC1`-`SYNC4`/`DATA` fields;
- notifications, crash reports, backups, or WorkManager input data;
- Rust/Kotlin debug strings or normal sync logs.

The Rust FFI receives an opaque account handle and asks the credential broker
for the secret only inside a bounded sync operation. The Kotlin UI receives
redacted status and error categories, never the credential. Keystore failure,
credential loss, or account/session invalidation is a recoverable
`credential-blocked` state; it must not trigger destructive provider cleanup.

## Scheduling and background execution

Use Android's sync-adapter framework for account-associated periodic work where
the supported Android range permits it. Use WorkManager as the durable job
orchestrator for bounded reconciliation, credential repair, migrations, and
recovery tasks. A provider observer is only a prompt signal; it is not a
durable queue.

Every scheduled run has:

- network constraint when Anytype access is required;
- an account-generation check;
- a short, bounded provider transaction batch;
- an idempotency key and Rust checkpoint;
- retry classification (transient, credential, permission, schema, conflict,
  permanent);
- a reschedule/backoff decision.

Do not promise immediate delivery under Doze, app standby, OEM task killers,
metered-network restrictions, or disabled system sync. On process restart,
WorkManager and the next sync callback resume from the last committed
checkpoint. A crash before checkpoint commit may repeat an operation, so all
creates must reconcile by canonical identity before insertion.

## Permissions and capability changes

Request only permissions for enabled projections:

| Projection | Runtime permissions | Failure behavior |
|---|---|---|
| Contacts | `READ_CONTACTS`, `WRITE_CONTACTS` | pause Contacts only |
| Calendar | `READ_CALENDAR`, `WRITE_CALENDAR` | pause Calendar only |
| Tasks.org | `org.tasks.permission.READ_TASKS`, `WRITE_TASKS` | pause Tasks only |

Permission grant is not proof that a column or provider operation exists.
Probe the authority, package/version, required columns, and write semantics
before enabling each adapter. On revocation, stop reads and writes before
repair; preserve checkpoints and mappings, show a repair state, and never
interpret an empty query as a remote deletion.

## Provider-loop prevention

Any-Cal-generated provider changes must not be fed back as new Anytype edits.
For each operation record:

```text
operation ID
account generation
canonical object ID / DAV UID
provider row ID and source ID
pre-write hash
post-write hash
origin = anytype | provider | merged
checkpoint
```

Contacts writes append `CALLER_IS_SYNCADAPTER` and use the account-owned raw
contact. Calendar writes use the account and sync-adapter URI parameters. The
post-write provider hash is compared with the expected projection hash. An
observer callback with the expected hash is an echo; a differing hash is an
external edit and enters conflict handling.

Observers are debounced and coalesced. They query only Any-Cal-owned rows,
using `SOURCE_ID`/`_SYNC_ID` and dirty/version metadata where available. They
must never scan all provider rows and infer ownership from names, colors, or
display labels.

## EteSync and DAVx⁵ coexistence

EteSync/DAVx⁵ rows are separate account/provider ownership domains. Any-Cal
must not update them or reuse their account type. If the same Anytype objects
are simultaneously projected by Any-Cal and EteSync/DAVx⁵, duplicate rows and
feedback loops are expected risks.

The setup UI must choose exactly one owner for a dataset on a device:

- **Direct mode:** Any-Cal owns the Contacts/Calendar account and optional
  Tasks.org projection.
- **DAV fallback:** EteSync/DAVx⁵ owns the Android projection; Any-Cal exposes
  the DAV service for clients and does not also write those rows directly.

An import wizard may read another account and create Any-Cal-owned rows, but
must report that this is a copy/import, preserve source identifiers, and never
silently merge or delete the source account. Android contact aggregation may
make two raw contacts appear as one aggregate contact; the raw-contact account
and source ID remain authoritative.

## Checkpoints, tombstones, and provider recreation

Checkpoint state is per account generation and provider adapter:

```text
(space ID, account generation, authority, adapter version, capability hash,
 last Anytype cursor, last provider scan marker, operation sequence)
```

Tombstones retain canonical identity, deletion origin, and revision until all
enabled projections acknowledge deletion or the retention policy expires.
Missing rows are classified before action:

- expected provider deletion;
- account removal;
- permission loss;
- provider database recreation/restore;
- adapter mapping corruption.

Only the first case is an ordinary delete signal. For the others, pause and
rebuild through an explicit reconciliation mode. Recreated provider rows are
matched by stable source identity before projection; never use local row IDs as
canonical IDs.

## API, OEM, and release gates

Before provider writes are enabled on a release/device family, the disposable
test matrix must prove:

- account add/remove/re-add and process restart;
- permission grant, revocation, and re-grant;
- offline edits, network loss, retry, and crash recovery;
- Doze/standby delay is tolerated;
- Contacts raw-contact ownership, aggregation, groups, and source IDs;
- Calendar account ownership, recurrence, reminders, and timezone behavior;
- provider database recreation does not duplicate records;
- Tasks.org authority/version/permission/column probe, if enabled;
- direct mode and DAV fallback cannot both own the same configured dataset.

Any OEM-specific failure is a capability result, not a reason to broaden
permissions or delete/recreate all rows. Disable only the affected adapter and
retain the DAV fallback.

## Unresolved decisions

1. Whether to use Android periodic sync-adapter scheduling, WorkManager, or
   both for each minimum API level.
2. Exact account-removal behavior across stock Android and target OEM builds.
3. Keystore/backup policy for credentials when Android backup restores the
   app without provider rows.
4. Whether Tasks.org's unstable provider is acceptable as a supported target,
   and the minimum tested version.
5. Whether a one-time import from EteSync/DAVx⁵ is worth the aggregation and
   duplicate-management cost.

Until these gates pass, this design supports capability probing and one-way
projection planning only; it does not claim live sync reliability.
