# Local DAV scheduling and attendee contract

Status: complete for provider-free synthetic coverage. This receipt does not
claim mail delivery, scheduling transport, or interoperability with external
clients.

## Scope and safety boundary

The lane used only in-process Rust tests with synthetic VEVENT/VTODO bodies.
It did not access the network, Anytype, Android/provider state, DAVx5,
Tasks.org, the personal Flatpak, credentials, or the pending credential
artifact. Temporary test state was in memory and left no external resources.

## Verified behavior

- `ical::Calendar` preserves repeated `ATTENDEE` properties, organizer and
  attendee parameters (`CN`, `RSVP`, `PARTSTAT`, `ROLE`, `CUTYPE`), and
  scheduling metadata (`STATUS`, `SEQUENCE`, `DTSTAMP`) across parse/serialize
  for synthetic `VEVENT` data.
- The VTODO projection updates editable `STATUS` while retaining opaque
  `ORGANIZER`/`ATTENDEE` values and their parameters. Serialization and
  reparsing preserve both the update and scheduling metadata.
- Empty and unterminated attendee parameter values are rejected fail-closed;
  no replacement document is produced by the parser.
- A local CalDAV VTODO PUT/GET preserves organizer, repeated attendees, RSVP,
  participation status, role, cutype, status, sequence, and timestamp. The
  server emits an ETag and `Last-Modified` value.
- A malformed attendee update returns `400` and leaves the existing body,
  ETag, and `Last-Modified` unchanged. No repository mutation occurs on this
  failure path.

## Evidence

Commands, with `LD_PRELOAD` unset:

```text
cargo fmt --all
cargo test -p any-cal-core --test ical --test roundtrip
  5 + 9 tests passed
cargo test -p any-cal-dav-server --test matrix scheduling_attendees_round_trip_and_invalid_update_does_not_mutate -- --exact
  1 test passed
cargo clippy -p any-cal-core -p any-cal-dav-server --tests -- -D warnings
  passed
```

The complete DAV matrix was not rerun in this lane; existing matrix evidence
remains separate. External scheduling interoperability remains an explicit
follow-up requiring authority and real-client fixtures.
