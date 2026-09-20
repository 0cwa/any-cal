# Local component-aware DAV contract

Date: 2026-09-20

This receipt records a bounded, offline synthetic check. It did not access
Anytype, Android, DAVx5, Tasks.org, external network state, the personal
Flatpak, or the pending credential artifact. No credential-bearing file was
read.

## Focused implementation

`any-cal-core::ical` now provides a wire-preserving iCalendar component tree:

- `VCALENDAR` is required as the sole root;
- `VEVENT`, `VTODO`, `VALARM`, `VTIMEZONE`, `STANDARD`, and unknown components
  retain their nesting, property ownership, and input order;
- repeated properties retain occurrence order and per-occurrence parameters;
- mismatched, missing, extra, or non-calendar roots are rejected;
- serialization uses deterministic CRLF boundaries and the existing property
  escaping rules, so canonical bytes produce stable ETags.

The existing Anytype-facing VTODO projection remains intentionally flat and
VTODO-only. The component tree is a separate preservation seam; it does not
claim that the current DAV server exposes VEVENT collections or full calendar
scheduling.

## Evidence

Focused tests in `crates/core/tests/ical.rs` passed:

- mixed `VTIMEZONE`/`STANDARD`, `VEVENT`/`VALARM`, `VTODO`, and unknown
  component/property ownership and ordering;
- canonical parse/serialize idempotence and stable ETag;
- repeated parameterized properties with quoted commas/semicolons;
- malformed boundary, multiple-root, non-calendar-root, and property-outside-
  component rejection.

Commands (with `LD_PRELOAD` unset):

```text
cargo fmt --all -- --check                         PASS
cargo clippy -p any-cal-core --all-targets --offline -- -D warnings  PASS
cargo test -p any-cal-core --offline               PASS (25 tests)
cargo test -p any-cal-dav-server --offline         PASS (36 tests)
```

## Boundary and follow-up

The DAV server's collection query/multistatus implementation remains truthful
for its supported Contact/VTODO profile. It does not yet persist the component
tree through the repository envelope, so this lane does not claim mixed
VEVENT HTTP interoperability. A future event-support lane must decide how to
persist the ordered tree and then add server-level mixed-component REPORT and
multistatus tests before advertising VEVENT capability.
