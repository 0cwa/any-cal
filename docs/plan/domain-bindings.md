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

A future multi-domain configuration can model:

```text
/dav/contacts/personal        -> personal-contacts -> space-personal
/dav/contacts/mutual-friends  -> mutual-friends    -> space-friends
/dav/tasks/personal           -> personal-tasks    -> space-personal
/dav/calendars/mutual-events  -> mutual-events     -> space-events
```

The contract permits the shared `space-personal` target because route identity remains
domain-qualified.

## Current implementation boundary

The contract lives in `any-cal-core` and current `AppConfig` can project its scalar
configuration to a validated legacy binding set. Runtime repository routing and durable
sync/checkpoint qualification are intentionally separate work tracked in GitHub issue
#3. Multi-domain external configuration should not be advertised as operational until
that routing work is complete.
