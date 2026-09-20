# Local DAV query/filter/REPORT semantics

Date: 2026-09-20
Scope: offline synthetic repository only.

## Result

The bounded CardDAV/CalDAV matrix passes 26 tests in
`crates/dav-server/tests/matrix.rs`. The focused implementation fixes were:

- DAV XML element prefixes are normalized before the bounded parser runs, so
  standard namespace-qualified `c:`/`d:` REPORT elements are accepted without
  treating namespace prefixes as meaningful data.
- `comp-filter` names are parsed and checked against the persisted opaque
  calendar component tree. A task collection therefore matches `VTODO` and
  does not falsely match a `VEVENT` filter; synthetic rows without an opaque
  tree use their collection kind as the fallback.
- Empty self-closing `prop-filter` elements are treated as property-presence
  filters, as required by the query shape, rather than being rejected as
  malformed.

The matrix covers addressbook-query/multiget, calendar-query/multiget,
property selection, text and time-range filters, component filters, missing
multiget hrefs, malformed XML/filter reports, escaped href/data values,
deterministic repository ordering, depth handling, content types, and truthful
CardDAV/CalDAV capability advertisements. Existing HTTP framing/error tests
remain separate and were not broadened here.

## Verification

Commands, with `LD_PRELOAD` explicitly unset:

```text
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test matrix -- --nocapture
26 passed, 0 failed
env -u LD_PRELOAD cargo fmt --check
pass
env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --all-targets -- -D warnings
pass
```

No external network, DAV client, Anytype state, Android/provider state,
personal Flatpak, credential, or pending credential artifact was accessed.

## Remaining boundary

This is a local protocol-semantic receipt, not proof of interoperability with
DAVx5, Tasks.org, or other external clients. Component matching is intentionally
bounded to the persisted tree and supported VTODO collection; broader CalDAV
component semantics remain a separate research/compatibility lane.
