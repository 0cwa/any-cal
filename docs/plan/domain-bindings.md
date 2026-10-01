# Domain-to-Space binding contract

Any-Cal routes each **canonical DAV domain** to a fixed Anytype Space and credential
profile. The binding contract is versioned separately from runtime configuration so
routing, sync identity, schema discovery, migration, and onboarding share one durable
model.

A binding answers only: **where does this canonical DAV domain live?**

Personal cross-Space composition is intentionally a separate contract. See
[cross-space-composition.md](cross-space-composition.md).

## Contract

The current schema version is `1`.

A `DomainBinding` contains:

- `domain_id`: stable machine identifier for the canonical local domain;
- `label`: human-readable display label;
- `space_id`: fixed upstream Anytype Space;
- `credential_profile_id`: external credential-profile reference, never a raw
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

One domain may own multiple routes. The legacy compatibility binding owns the existing
CardDAV contact route and CalDAV task route together. New deployments may instead use
one domain per address book, task list, or calendar.

## Invariants

- A contract contains at least one binding.
- Domain IDs are unique.
- Route paths are globally unique, so an inbound path resolves to one binding.
- Multiple domains may intentionally target the same Space.
- Request data never supplies or overrides the Space.
- Collection/component mismatches fail closed.
- Unknown JSON fields fail closed.
- Serialization is normalized by domain ID and route order.
- The binding fingerprint changes when canonical routing identity changes, including
  Space, credential profile, upstream account fingerprint, schema profile, checkpoint
  namespace, or route set.
- Lifecycle state is not part of the fingerprint; temporary access failure does not
  create a new durable identity.
- Personal composition preferences do not belong in this contract and do not change a
  canonical binding fingerprint.

## Boundary with cross-Space composition

A `DomainBinding` identifies the home of a canonical DAV resource. It does **not**
model:

- a personal Daily Plan that reads several calendar domains;
- a private Person Context attached to a shared Contact;
- a materialized reference cached in another Space;
- native or synthetic cross-Space object relations;
- a list of Spaces that a request may choose dynamically.

Those are composition concerns.

Composition uses configured domain IDs as authorized source/destination handles and
stores a durable foreign identity equivalent to account + source Space + source object.
It must not reinterpret a foreign object as canonical merely because it is materialized
into another Space.

This separation is important for recovery: changing a personal composition profile
must never invalidate canonical DAV checkpoints or make a missing materialized
reference look like deletion of its canonical source.

## Runtime resolution

The target runtime shape is:

```text
request path
  -> DomainBindings.resolve_route(path)
  -> DomainRepositoryRegistry[domain_id]
  -> binding-qualified repository operation
```

A normal DAV operation resolves exactly one context. The long-term runtime must not
depend on a process-global mutable current Space.

A composition operation may explicitly read several authorized domain contexts and
write one configured private/home destination context, as defined in
[cross-space-composition.md](cross-space-composition.md).

## Legacy compatibility

The scalar configuration:

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

This preserves single-Space behavior while allowing explicit bindings to replace
scalar routing without inventing a second canonical identity model.

## Example

A multi-domain configuration can model:

```text
/carddav/personal         -> personal-contacts -> space-personal
/carddav/mutual-friends   -> mutual-friends    -> space-friends
/caldav/personal          -> personal-tasks    -> space-personal
/caldav/mutual-events     -> mutual-events     -> space-events
```

The contract permits multiple domains to share `space-personal` because runtime
identity remains domain/binding qualified.

A user's separate composition profile might then allow:

```text
source: mutual-friends, mutual-events
destination: personal
```

That profile is not serialized into `DomainBindings`.

## Implementation boundary

Core binding validation/fingerprinting and legacy projection are implemented in
`any-cal-core`. Application/runtime code is responsible for turning configured
bindings into independently scoped repository contexts before treating explicit
multi-domain routing as operational.

Cross-Space composition must be layered on top of those contexts, not added as another
Space-selection path inside DAV routing.
