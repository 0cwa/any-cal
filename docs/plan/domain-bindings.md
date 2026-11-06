# Domain-to-Space binding contract

Any-Cal routes each local DAV domain to a fixed Anytype Space and credential profile.
The binding contract is versioned separately from runtime configuration so routing,
sync identity, schema discovery, and later onboarding can share one durable model.

## Contract

The current schema version is `1`.

A `DomainBinding` contains:

- `domain_id`: stable machine identifier for the local domain;
- `label`: human-readable display label;
- `space_id`: fixed upstream Anytype Space;
- `credential_profile_id`: reference to an external credential profile, never a raw
  credential;
- `routes`: one or more fixed DAV routes owned by the domain;
- `schema_profile`: schema/capability profile expected in the Space;
- `checkpoint_namespace`: durable local sync/checkpoint namespace;
- `visibility`: `private`, `shared`, or `unknown`;
- `lifecycle`: configured/verified/access-denied/stale/migration-required/disabled.

A route records its logical collection, DAV component, and exact path. The component
must match the collection:

| Collection | Component |
| --- | --- |
| Contacts | vCard |
| Tasks | VTODO |
| Events | VEVENT |

One domain may own multiple routes. This is required for the legacy compatibility
binding, which owns the existing CardDAV contact route and CalDAV task route together.
New deployments can instead use one domain per contact book, task list, or calendar.

## Invariants

- A contract must contain at least one binding.
- Domain IDs are unique.
- Route paths are globally unique, so an inbound path resolves to exactly one binding.
- Multiple domains may intentionally target the same Space.
- Request data does not supply or override the Space.
- Collection/component mismatches fail closed.
- Unknown JSON fields fail closed.
- Serialization is normalized by domain ID and route order.
- The binding fingerprint changes when routing identity changes, including Space,
  credential profile, upstream account fingerprint, schema profile, checkpoint
  namespace, or route set.
- Lifecycle state is not part of the fingerprint; a temporary access failure must not
  create a new durable identity.

## Boundary with cross-Space composition

A `DomainBinding` answers only where a canonical DAV domain lives. It does not
encode a principal's private composition preferences, a Personal Context, an Event
Reference, a Daily Plan, or a dynamic list of Spaces a request may choose.

Composition is a separate principal-scoped contract. It resolves configured domain IDs
through the repository registry, may read several already-authorized source contexts,
and writes exactly one explicitly configured private/home destination context. Request
data never supplies or overrides source/destination Space IDs.

The durable identity of a foreign source object is upstream account context + source
Space ID + source object ID. A binding fingerprint remains routing/configuration
identity and may legitimately change without making the foreign object a new object.

Changing a composition profile therefore must not alter canonical DAV resource identity,
binding fingerprints, checkpoint namespaces, or migration state. See
[cross-space-composition.md](cross-space-composition.md).

## Legacy compatibility

The existing scalar configuration:

```text
space_id
contacts_collection
tasks_collection
```

projects to one binding named `legacy-default` with:

```text
/carddav/<contacts_collection>
/caldav/<tasks_collection>
```

This preserves the current single-Space behavior while allowing later work to migrate
configuration to explicit bindings without inventing a second routing model.

## Example

The current Contacts/VTODO runtime can route, for example:

```text
/carddav/personal        -> personal-contacts -> space-personal
/carddav/mutual-friends  -> mutual-friends    -> space-friends
/caldav/personal         -> personal-tasks    -> space-personal
/caldav/team-tasks       -> team-tasks        -> space-team
```

The contract permits the shared `space-personal` target because route identity remains
domain-qualified.

## Current implementation boundary

The core contract and explicit multi-domain application configuration are operational
for Contacts and VTODO routes. Use `domain_bindings` in a config file,
`ANY_CAL_DOMAIN_BINDINGS_JSON`, or `--domain-bindings-json`. Explicit bindings take
precedence over scalar legacy settings and resolve through `DomainRepositoryRegistry`
to independently scoped repository contexts.

Current application limits are deliberate and validated before startup:

- All bindings use the one configured upstream credential profile; an unknown profile
  is rejected.
- Contacts use `/carddav/<collection>` and tasks use `/caldav/<collection>`, with exactly
  one collection segment and at most one route of each supported kind per binding.
- The core contract can describe VEVENT routes, but the application rejects them until
  event lifecycle/query semantics are implemented in issue #7.
- Multi-domain configuration with the single `sync_checkpoint` option is rejected.
  Binding-qualified checkpoint primitives do not yet provide a multi-domain checkpoint
  configuration surface.

The scalar fields remain an intentional single-domain input that converts to
`legacy-default`; they are never a fallback for an explicit route. Canonical migration
and recovery are tracked in issue #8. Cross-Space composition remains a separate
principal-scoped operation and cannot override these routing rules.
