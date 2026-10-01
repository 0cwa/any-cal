# Anytype DAV adapter plan

> **Documentation status:** This is a durable architecture document. Start at
> [../README.md](../README.md) for the project documentation map. Files in this
> directory ending in `-current.md` are historical evidence snapshots unless a
> durable document links to them for a specific claim.

## Purpose

Any-Cal is an always-on CardDAV and CalDAV server whose durable application data lives
in one or more explicitly bound Anytype Spaces. It translates canonical DAV resources
to Anytype objects and translates those objects back to vCard/iCalendar on request.
It does not use a second durable sync database.

The current desktop client is a thin Slint setup/status/diagnostics UI; it is not the
DAV implementation. Tauri 2 + Svelte/TypeScript was researched as an alternative, but
is not a dependency or supported build target in this repository.

The service defaults to the Anytype HTTP transport. A listener is only local
configuration readiness: `/health` reports whether the upstream has been tested, while
`/ready` performs bounded configured-Space reads before reporting readiness. The fake
transport is explicit fixture/development mode and is never a production fallback.

Contacts are exposed through CardDAV. Tasks are exposed as CalDAV `VTODO` resources.
Calendar events are a later `VEVENT` profile.

## Non-negotiable invariants

- No separate persistent sync database. Short-lived caches/checkpoints may exist only
  under the documented recovery model; canonical user data lives in Anytype.
- Every canonical DAV domain maps to exactly one Anytype Space through a configured
  domain binding. Requests never choose/override a Space.
- An Anytype object is the canonical resource inside its owning Space. Its Anytype ID
  determines the DAV resource URL; DAV `UID` is retained as object data.
- Runtime identity, caches, queues, checkpoints, tombstones, and schema/capability
  state are binding-qualified so identical object IDs/UIDs in different Spaces cannot
  collide.
- A complete structured representation of unprojected DAV data is intended to be stored
  with the canonical object. Normal Anytype properties are useful user-editable
  projections, not lossy replacements for canonical DAV semantics.
- Cross-Space personal views use explicit composition/materialized references; they do
  not turn a canonical object into a multi-Space object or create direct foreign-Space
  Anytype relations.
- Source-owned cached fields and destination-user-owned private fields have explicit
  ownership. Refresh must never overwrite private facet content.
- The server advertises only capabilities it implements and tests against real clients.

## Target architecture

```text
DAV clients
    |
    v
Any-Cal headless service
  - authentication/authorization
  - DAV routing and protocol handling
  - DomainRepositoryRegistry
        domain A -> binding -> Anytype Space A
        domain B -> binding -> Anytype Space B
        domain C -> binding -> Anytype Space C
  - mapping/canonical DAV envelopes
  - optional personal composition
        read authorized source contexts
        -> materialize bounded reference/facet
        -> one private/home destination context
    |
    v
Anytype Spaces (canonical + private derived objects)

Slint UI: setup, mappings, composition preferences, status, diagnostics
```

Normal DAV requests resolve one canonical domain context. Composition is an explicit
read-many/write-one operation and must not depend on a mutable process-global selected
Space.

See [domain-bindings.md](domain-bindings.md) for canonical routing,
[cross-space-composition.md](cross-space-composition.md) for private facets and Daily
Plans,
[schemas.md](schemas.md) for object models,
[development-efficiency.md](development-efficiency.md) for thin-slice sequencing,
[research.md](research.md) for data/protocol decisions,
[android-research.md](android-research.md) for the Android provider-source boundary,
[etesync-research.md](etesync-research.md) for EteSync/Etebase modes, and
[architecture-research.md](architecture-research.md) for language/server/UI decisions.

## Complete enough to implement

The direct CardDAV + CalDAV VTODO slice is specified well enough to implement.
Multi-Space canonical routing is staged behind binding-qualified isolation. Personal
cross-Space composition has a separate durable contract and is implemented only after
the repository-context and authorization seams can enforce it safely.

Android provider import, Etebase projection, VEVENT, recurrence, materialized Person
Contexts, Daily Plans, and richer mapping UI remain extension seams. A decision is
blocking only where the durable design/research register says so; unsupported
capabilities must not be advertised.

## Phased delivery

1. Canonical routing foundation: explicit domain-to-Space bindings, binding-qualified
   repository/sync state, multi-Space collision isolation, and repository-context
   registry.
2. Authorization/capability foundation: domain-scoped principal policy, upstream role
   state, read-only schema/member/view discovery.
3. Typed canonical schemas and relations inside one Space.
4. Cross-Space composition primitives and private Person Context proof.
5. CalDAV VEVENT semantic correctness and Space-local Calendar View integration.
6. Deterministic private Daily Plans/Event References across configured source Spaces.
7. Android provider import fixture and, only if justified, companion/sync-adapter work.
8. Incremental sync, richer recurrence/alarms/attachments/client interop, migration UX,
   and controlled sharing/composition pilots.

Each phase ends with deterministic isolation tests and, where applicable, disposable
interoperability/two-principal validation before expanding advertised capabilities.
