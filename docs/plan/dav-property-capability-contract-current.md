# Local DAV property capability contract

Date: 2026-09-20

This lane used only `DavServer<MemoryRepository>` with synthetic contacts and
VTODO resources. No network, credentials, Anytype state, Android state,
Flatpak state, or pending credential artifact was accessed.

## Findings and fixes

- `PROPFIND` now normalizes client-selected XML prefixes before parsing, so
  `d:prop`/`d:getetag` and unprefixed forms select the same DAV property.
- `PROPFIND <propname/>` now returns the supported property vocabulary as
  empty elements. It no longer behaves like an empty explicit property list.
- Explicitly requested but unsupported properties are returned in a separate
  per-resource `404 Not Found` propstat; supported properties remain in a
  `200 OK` propstat.
- `REPORT` projections now apply the same bounded per-resource `404` handling
  for unknown requested properties.
- `allprop` continues to emit only properties implemented by the relevant
  collection/resource profile. Contacts do not advertise calendar reports;
  tasks advertise VTODO only.

## Evidence

Focused matrix: `41 passed` (`cargo test -p any-cal-dav-server --test matrix`).
Clippy: `cargo clippy -p any-cal-dav-server --tests -- -D warnings` passed.
Formatting: `cargo fmt --check` passed after formatting.

The synthetic matrix covers namespaced explicit property requests,
`propname`, `allprop`, unknown properties, CardDAV/CalDAV capability
separation, deterministic response generation, and bounded multistatus
status/content-type behavior. No external interoperability claim is made by
this receipt.
