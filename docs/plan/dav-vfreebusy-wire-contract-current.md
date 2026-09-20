# Local VFREEBUSY wire contract

Date: 2026-09-20

## Scope and boundary

This lane used only synthetic in-process iCalendar values. It did not use a
network listener, external DAV client, scheduling delivery, Anytype,
Android, DAVx5, Tasks.org, Flatpak state, credentials, or the pending
credential artifact. It does not implement recurrence expansion or timezone
database conversion.

## Implemented contract

`any_cal_core::vfreebusy::VFreeBusy` now provides a bounded local wire model:

- parses one `VFREEBUSY` component with `DTSTART`, `DTEND`, `ORGANIZER`,
  `URL`, `TZID`, and explicit `FREEBUSY` periods;
- preserves `FBTYPE` (`BUSY`, `BUSY-TENTATIVE`, `BUSY-UNAVAILABLE`, `FREE`,
  and deterministic opaque values);
- rejects malformed periods and duration-form periods rather than silently
  guessing their meaning;
- normalizes supported dates/date-times to the existing literal wall-clock
  free/busy representation;
- serializes stable CRLF output with deterministic type and interval order;
- projects by type through the existing half-open clip/merge helper;
- exposes deterministic content ETags and carries `ModifiedAt` metadata;
- enforces default 64 KiB wire and 512-period bounds, with configurable
  limits for both parsing and serialization/projection.

## Evidence

Commands were run with `LD_PRELOAD` unset:

```text
cargo fmt --check
cargo test -p any-cal-core --test vfreebusy --test freebusy --test ical --quiet
cargo test -p any-cal-core --quiet
cargo clippy -p any-cal-core --all-targets -- -D warnings
```

Results: the new VFREEBUSY suite passed (4 tests); existing focused free/busy
and iCalendar suites passed (3 and 7 tests); all core tests passed; and core
clippy passed with `-D warnings`.

The tests cover deterministic metadata round trips and ETags, per-type
clipping/merging, empty input, malformed input, duration rejection, and byte
and interval bounds. No claim is made about external scheduling delivery or
client interoperability.

## Remaining research boundary

RFC duration-period semantics, recurrence expansion, timezone conversion,
production monitoring, scheduling delivery, and external client
interoperability remain separate work.
