# Local DAV alarm, timezone, and free/busy semantics

Date: 2026-09-20

## Scope

This lane used only synthetic in-process iCalendar values. It did not use a
network listener, external DAV client, Anytype, Android, DAVx5, Tasks.org,
Flatpak state, credentials, or the pending credential artifact. No recurrence
expansion or scheduling delivery was attempted.

## Evidence

Commands run with `LD_PRELOAD` unset:

```text
cargo fmt
cargo test -p any-cal-core --test freebusy --test ical --quiet
cargo clippy -p any-cal-core --all-targets -- -D warnings
cargo test -p any-cal-core --quiet
cargo test -p any-cal-dav-server --test matrix --quiet
```

Results: the focused free/busy and iCalendar tests passed (3 + 7), all core
tests passed, all 44 DAV matrix tests passed, and core clippy passed with
`-D warnings`.

## Implemented bounded contract

- `any_cal_core::freebusy::project` clips half-open intervals to a requested
  window, subtracts sorted exclusions, merges overlaps, and returns stable
  normalized ordering.
- Empty windows and exact end-boundary intersections produce no phantom busy
  interval. Reversed intervals, invalid dates, and reversed windows fail
  closed.
- Date-only, floating, UTC (`Z`), and extended date-time values normalize to a
  basic wall-clock key. TZID-associated values remain literal wall-clock data;
  this layer performs no timezone-database conversion.
- Synthetic DST-boundary `VTIMEZONE`/`VEVENT` data retains its TZID and local
  values through parse/serialize/parse. Nested `VALARM` action, relative
  trigger, repeat, duration, and escaped description retain deterministic
  structure.
- Malformed alarm parameters, missing property separators, and unbalanced
  component boundaries are rejected without a replacement value.

The free/busy helper is intentionally not a recurrence engine and does not
advertise external scheduling or client interoperability.

## Remaining research boundary

Production timezone conversion (including ambiguous/nonexistent DST local
times), recurrence expansion, VFREEBUSY wire/report support, alarm delivery,
and client-specific semantics remain separate work. The current helper is
appropriate only for bounded literal interval projection.
