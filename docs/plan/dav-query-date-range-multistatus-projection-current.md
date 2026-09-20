# Local DAV date-range multistatus/property projection receipt

Date: 2026-09-20

## Scope and isolation

This lane used only provider-free in-process `MemoryRepository` fixtures and
the DAV matrix test target. It did not use a network listener, external DAV
client, Anytype, Android, DAVx5, Tasks.org, Flatpak state, credentials, or the
pending credential artifact. No external or destructive operation was run.

## Commands and results

    env -u LD_PRELOAD cargo test -p any-cal-dav-server --test matrix --quiet
    env -u LD_PRELOAD cargo fmt --all -- --check
    env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --all-targets -- -D warnings

All 39 DAV matrix tests passed. Formatting and Clippy passed with warnings
denied.

## Reproduced defect and fix

REPORT projection previously emitted only `getetag` and the requested
`address-data`/`calendar-data`. A client requesting DAV metadata such as
`getlastmodified`, `getcontenttype`, or `getcontentlength` received a 200
property response that silently omitted the requested fields. REPORT now
projects those fields when requested (and retains the existing all-properties
behavior when no property list is supplied). `getlastmodified` is derived from
the persisted `ModifiedAt`; content type and byte length are derived from the
selected current representation.

## Matrix evidence

- Calendar multiget with matching, archived, and missing synthetic resources
  returns a deterministic 207 multistatus. The live resource receives ETag,
  Last-Modified, content type, and content length; archived and missing hrefs
  receive truthful 404 statuses.
- Resource response ordering remains ResourceId/href order, including the
  per-resource error entries.
- A synthetic malformed repository read returns HTTP 500 rather than an empty
  successful multistatus, so backend corruption is not projected as absence.
- Existing date-range tests continue to prove inclusive-start/exclusive-end
  selection, UTC/floating normalization, empty results, malformed/reversed
  ranges, CardDAV fail-closed behavior, mixed component preservation, ETag
  stability, and ModifiedAt independence.
- Request and output fixtures remain bounded by the existing DAV parser/body
  limits; this lane did not add recurrence expansion, timezone conversion, or
  retention behavior.

## Remaining boundary

This receipt does not claim interoperability with external clients or live
providers. Such validation remains in the separate authority-gated lanes.
