# Cross-Space personal composition

> **Documentation status:** Durable architecture contract.
>
> Canonical implementation sequencing lives in GitHub issue #1. This document defines
> the design invariants that should remain stable even as individual issues/PRs move.

## Purpose

Any-Cal uses Anytype Spaces as canonical ownership and permission boundaries. That is
a good fit for shared contact books, calendars, and task lists, but users also need
private context around shared objects and personal views that combine several Spaces.

Examples:

- a shared Contact should remain shared while one user keeps private notes about it;
- a personal Daily Plan should be able to show events from personal, family, and work
  calendar Spaces without moving those events into one Space;
- a personal task or page should be able to relate richly to a shared Person/Event
  without requiring a native Anytype cross-Space object relation.

The solution is **composition**, not canonical duplication. Any-Cal may materialize a
bounded local reference/facet in an explicitly private/home Space while preserving the
foreign object's exact source identity.

## Architectural split

Keep two contracts separate:

```text
DomainBinding
  answers: where does this canonical DAV domain live?

CompositionProfile
  answers: which authorized source domains may feed this private/home context?
```

Changing a composition preference must not change a canonical binding fingerprint,
checkpoint namespace, DAV resource identity, or migration state.

A canonical DAV object always has one owning domain/Space. A materialized reference is
not a second canonical Contact, Task, or Event.

## Core terms

### DomainRepositoryRegistry

Runtime registry keyed by stable `domain_id`. Each entry owns the binding-qualified
repository/cache/collection context for one canonical domain.

Normal DAV routing resolves to exactly one context. Composition explicitly obtains
several readable source contexts plus one writable destination context.

There must be no process-global mutable "current Space" required for request
correctness.

### ForeignObjectRef

Durable identity for an object outside the destination Space.

Conceptually:

```text
ForeignObjectRef {
  upstream_account_fingerprint
  source_space_id
  source_object_id
  kind
  dav_uid? // optional correlation metadata
}
```

The identity is **account + source Space + source object**. A binding fingerprint is
not the permanent foreign-object identity because route/configuration changes can alter
the binding fingerprint without changing the upstream object.

DAV UID, display name, email address, or title must not be used alone to retarget a
foreign reference.

### CompositionProfile

Principal-scoped configuration describing:

- the explicitly selected private/home destination domain;
- the configured source domains that may be read for composition;
- enabled composition features/projection policy;
- optional presentation preferences that do not affect canonical routing.

The profile is separate from `DomainBindings`.

### MaterializedReference

A destination-Space object containing:

- `ForeignObjectRef`;
- bounded source-owned cached properties needed for useful local views/relations;
- source availability/staleness metadata;
- optional source-open/deep-link metadata when a stable mechanism is available.

A materialized reference is rebuildable from source data plus destination-user-owned
facet data. It is not permission to mutate the source object.

### PersonalFacet

Destination-user-owned context attached to a foreign object. Examples include private
notes, personal tags, follow-up state, local relations, or annotations.

A PersonalFacet and MaterializedReference may be one physical Anytype object, but field
ownership must remain explicit.

## Field ownership

Every projected field belongs to one of these classes:

| Class | Examples | Refresh behavior |
| --- | --- | --- |
| Source-owned | cached name, event title/start/end/location | Any-Cal may refresh from canonical source |
| Destination-user-owned | private notes/body/tags/local relations | Never overwritten by source refresh |
| Derived | source status, stale marker, display summary | Recomputed deterministically |
| Local override | customized local title/label | Preserved once explicitly changed |

A useful title rule is:

- if the reference title still equals the last source-derived title, a source rename
  may update it;
- once the user explicitly overrides the local title, refresh preserves the override.

Schema/property mismatch must never cause Any-Cal to coerce a destination-user-owned
field into a source-owned field.

## Privacy and disclosure policy

Materialization copies information between permission domains. Treat it as an explicit
disclosure, even when the copy contains only a title or date.

Initial automatic policy:

1. destination must be an explicitly configured private/home domain for the principal;
2. every source domain must already be readable by that principal;
3. the principal must explicitly allow that source domain in the CompositionProfile;
4. unknown or broader destination audience relationships fail closed;
5. never use a request-supplied Space ID to select a source or destination.

This conservative policy can later be generalized if member/capability discovery can
prove an audience-subset relationship. It must never be generalized from display names
or assumptions such as "Personal" meaning private.

## Authorization model

Composition requires all of:

```text
local principal may read source domain
AND upstream credential may read source Space
AND local principal may write destination domain
AND upstream credential may write destination Space
AND composition profile explicitly permits source -> destination
```

Global Anytype search is discovery/search convenience only. A search result never
grants access to a domain and never selects a write destination.

## Runtime model

Canonical DAV request:

```text
request path
  -> DomainBinding
  -> DomainRepositoryRegistry[domain_id]
  -> one repository operation
```

Composition operation:

```text
configured source domain contexts (read-only)
  -> deterministic composer
  -> exactly one configured destination context (write)
```

Composition must not rely on mutable active-domain switching. This matters both for
concurrent DAV requests and for a single operation that needs multiple source Spaces.

## Lifecycle and revocation

### Source update

Refresh only source-owned/derived fields. Preserve destination-owned fields.

### Source archive/delete

Do not silently delete personal context. Mark the source status and follow an explicit
retention policy. A user may choose to delete the facet/reference separately.

### Source access loss

Mark the source unavailable/stale and stop refresh. Preserve:

- destination-user-owned content;
- exact `ForeignObjectRef`;
- a clearly stale last-known source snapshot if the configured policy retains one.

Reauthorization reconnects only when the exact account/Space/object identity is
validated.

### Destination deletion

Deleting the local reference/facet deletes only that destination object unless a
separate canonical operation is explicitly authorized. It never implies source delete.

### Canonical migration

Moving/copying a canonical object to another Space creates a new source identity.
Reference repair must use explicit migration provenance. Do not silently retarget by
DAV UID, name, email, or title alone.

## Shared contact + private notes

The first composition proof is a private Person Context.

```text
Shared contacts Space
  Carol Smith                 canonical CardDAV Contact

Alice private Space
  Carol · Personal Context    local reference/facet
    source_ref
    cached source name/email/phone
    private notes
    private tags
    follow-up state
    private relations

Bob private Space
  Carol · Personal Context    independent reference/facet
```

Alice's and Bob's contexts may share the same `ForeignObjectRef` source identity but
are separate destination objects. Neither user's private fields may be written into the
shared Contact or the other user's Space.

The Person Context is **not** exposed as an additional CardDAV contact by default.
Correctness must not depend on client-side contact deduplication.

A future virtual/composed address book would need a separate design for field
provenance, three-way diff, and safe split writes before it can accept whole-vCard PUTs.

## Daily Plan + Event References

The second composition proof is a deterministic Daily Plan in the principal's
private/home Space.

```text
Personal Space
  Daily Plan · 2026-09-23
    -> Event Reference: Team meeting
    -> Event Reference: School pickup
    -> Event Reference: Dinner reservation

Work Space
  Team meeting                canonical VEVENT

Family Space
  School pickup               canonical VEVENT
  Dinner reservation          canonical VEVENT
```

Use an idempotent operation equivalent to:

```text
ensure_daily_plan(principal, home_domain, local_date)
```

Correctness must not depend on the service being alive at midnight. Reconciliation can
run on startup/reconnect, a scheduler, or first access and must converge on one page for
principal + destination domain + local date + schema version.

### Calendar authority

Daily-plan membership comes from canonical VEVENT semantics, including:

- UTC/floating/TZID timestamps;
- local time zone and DST boundaries;
- all-day dates;
- `DTEND`/`DURATION`;
- recurrence;
- `RDATE`, `EXDATE`, exceptions;
- multi-day overlap;
- explicit cancelled/archived policy.

Projected Anytype date properties are useful indexes/presentation fields, but they are
not authoritative for recurrence or lossless time-zone interpretation.

Event References cache only the bounded source fields needed for useful local
relations/views. Personal annotations remain destination-owned.

## DAV boundary

Default DAV behavior remains canonical:

- canonical Contacts appear in their configured CardDAV collection;
- canonical Tasks/Events appear in their configured CalDAV collection;
- Person Contexts/Event References do not become duplicate DAV resources.

This boundary prevents an Anytype-oriented composition feature from changing DAV client
semantics unexpectedly.

## Anytype API posture

Verified against the Anytype developer documentation on 2026-09-23:

- stable API v1 (2025-11-08) provides global search across accessible Spaces;
- object listing/CRUD also remains Space-addressed;
- API v2 is explicitly pre-release and may change without a new API version.

Therefore:

- use configured domain contexts as authorization/routing truth;
- use global search only as an optional lookup optimization;
- keep production contracts on stable v1 until a deliberate version migration;
- capability-gate any v2 experiment.

References:

- https://developers.anytype.io/docs/reference/2025-11-08/search-global/
- https://developers.anytype.io/docs/reference/2025-11-08/list-objects/
- https://developers.anytype.io/docs/reference/v2/

Do not assume the Anytype UI can render native cross-Space relations/queries merely
because the API can search across Spaces.

## Minimum deterministic tests

Before a composition pilot, cover:

- identical source object IDs in two Spaces;
- source rename/update;
- source archive/delete;
- source permission revocation and reauthorization;
- destination deletion;
- local title override;
- restart/idempotent rematerialization;
- two principals materializing the same shared source;
- no private-field leakage between principals;
- removal of one source domain from a CompositionProfile;
- canonical Space migration with explicit reference repair;
- attempts to materialize into a shared/unknown-audience destination;
- concurrent source refresh and destination private edit.

Daily Plan additionally covers local-date boundaries, recurrence, exceptions, DST,
all-day, and multi-day events.

## Staging

Implementation is tracked in the roadmap issue and dedicated issues:

1. operational multi-domain canonical routing/collision isolation;
2. explicit repository-context registry;
3. composition primitives/privacy contract;
4. schema/capability support;
5. Person Context proof;
6. VEVENT semantic correctness;
7. Daily Plan/Event Reference proof;
8. onboarding and disposable two-principal validation.
