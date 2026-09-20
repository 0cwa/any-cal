# Local DAV recurrence, timezone, attendee, and alarm contract

Date: 2026-09-20

This is a local, deterministic synthetic validation. It did not access
Anytype, Android, DAVx5, Tasks.org, external network state, the personal
Flatpak, or the pending workspace credential artifact. No credential-bearing
file was read.

## Reproduced defects and focused fixes

The synthetic VTODO matrix reproduced two wire-preservation defects:

1. Quoted parameter values were split at commas and semicolons. For example,
   `ATTENDEE;CN="Doe, Jane"` and `ORGANIZER;CN="Planner; Team"` did not
   retain one parameter value through parse/serialize.
2. Calendar structured values were escaped as ordinary text. `RRULE` became
   `FREQ=WEEKLY\;BYDAY=MO\,WE...` and `EXDATE` dates were escaped, changing
   recurrence semantics on the wire.

The core parser now tokenizes quoted parameter values, removes syntactic
quotes, rejects unterminated quotes, and re-quotes values when serialization
requires it. Calendar list/structured properties (`CATEGORIES`, `EXDATE`,
`FREEBUSY`, `RDATE`, and `RRULE`) retain their delimiters.

## Coverage and evidence

The focused matrix passed:

- 6 `any-cal-core` round-trip tests, including UTC, floating, TZID, date/time
  forms, RRULE, RDATE, EXDATE, repeated attendee/organizer parameters,
  `PARTSTAT`, `STATUS`, `VALARM`, `TRIGGER`, opaque fields, and malformed
  quoted parameters;
- 20 direct DAV-server matrix tests, including synthetic recurring VTODO
  PUT/GET, deterministic serialization, ETag stability, conditional update,
  stale ETag rejection, and all supported timezone forms;
- all 7 DAV-server unit tests, 2 Android-profile fixture tests, 4 HTTP tests,
  and 3 replay tests;
- `cargo clippy -p any-cal-core -p any-cal-dav-server --all-targets --offline
  -- -D warnings`;
- `cargo fmt --all -- --check`.

## Explicit boundary

The current local CalDAV route stores and serves `VTODO`; it does not expose a
separate `VEVENT` collection or a full iCalendar component tree. The tests
therefore prove VTODO recurrence/attendee/alarm preservation and the generic
property envelope, not external-client interoperability or full VEVENT
scheduling semantics. VEVENT/component-aware support remains a separate
research and implementation lane.

