# Local DAV query/date-range interoperability receipt

Date: 2026-09-20

## Scope and evidence

This was a provider-free, in-process run using synthetic MemoryRepository
resources only. It did not use a network listener, external DAV client,
Anytype, Android, DAVx5, Tasks.org, Flatpak state, credentials, or the
pending credential artifact.

Commands:

    env -u LD_PRELOAD cargo test -p any-cal-dav-server --test matrix -- --quiet
    env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --all-targets -- -D warnings
    env -u LD_PRELOAD cargo test -p any-cal-core --quiet

Results: 37 DAV matrix tests, 29 focused DAV tests before this lane's
additions plus 8 new query/date-range cases, all passed; clippy passed with
-D warnings; all core tests passed.

## Implemented contract

- Calendar property time ranges use an inclusive start and exclusive end.
- Basic and extended date/date-time forms are normalized to a sortable
  wall-clock key. UTC (Z) and floating/TZID values are compared without
  timezone conversion; this preserves the literal local value and avoids
  inventing timezone rules.
- Date values are validated for shape, month/day, leap-year, and
  hour/minute/second bounds. Reversed or equal start/end bounds are rejected.
- A malformed time range returns HTTP 400. A valid range with no matching
  component returns deterministic HTTP 207 with no resource responses.
- CardDAV addressbook property filters reject time-range; CardDAV has no
  addressbook time-range semantics, so the server fails closed instead of
  broadening the query.
- ModifiedAt remains resource metadata for HTTP date/precondition behavior;
  component date filters use DTSTART, DTEND, DUE, or another requested
  component property. Updating a resource changes its revision/ETag and
  Last-Modified independently; the same date-range selection remains based
  on the component date.
- Repository iteration is ordered by ResourceId, and REPORT output retains
  deterministic href/property ordering. Requested ETags are emitted from the
  selected current representation.

## Regression cases

Synthetic VTODOs covered:

- exact inclusive start boundary;
- exact exclusive end boundary;
- UTC and floating values in one range;
- an empty but valid range;
- malformed calendar date;
- reversed range;
- unsupported CardDAV range;
- modified-at change independent from component-date selection and current
  ETag projection.

No full recurrence expansion, timezone conversion, or automatic retention
policy was introduced by this lane.

