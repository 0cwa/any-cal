# Local DAV date-precondition matrix — current receipt

Status: complete for the bounded provider-free lane on 2026-09-20.

## Scope and isolation

The run used only `MemoryRepository` resources and in-process `DavServer`
requests. It did not access external clients or networks, Anytype, Android,
DAVx5, Tasks.org, the personal Flatpak/device, credentials, or the pending
credential artifact. `LD_PRELOAD` was unset for every Rust command.

## Coverage

The matrix now covers:

- `GET`/`HEAD` with exact, older, future, malformed, and invalid-calendar
  `If-Modified-Since` values;
- `PUT` and `DELETE` stale/future `If-Unmodified-Since` values, including
  no-mutation on rejected DELETE and successful deletion with a future date;
- ETag precedence and wildcard behavior through the existing DAV matrix,
  including `If-None-Match` overriding a date condition and `If-Match` taking
  precedence for mutations;
- backward synthetic clock movement and monotonic modified-at behavior;
- collection `REPORT` behavior with date headers: reports remain `207` and
  are not incorrectly converted into representation-level `304` responses;
- serialized resource reopen preserving `Last-Modified` metadata and ETag
  identity through the core repository test.

Malformed and out-of-range dates are ignored as specified by the local
contract. Rejected writes return `412` before repository mutation. No defect
was reproduced, so no production code change was needed; one focused matrix
test was added.

## Evidence

```text
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test matrix -- --nocapture
34 passed, 0 failed

env -u LD_PRELOAD cargo test -p any-cal-core --test repository -- --nocapture
8 passed, 0 failed

env -u LD_PRELOAD cargo clippy -p any-cal-core -p any-cal-dav-server --all-targets -- -D warnings
passed

env -u LD_PRELOAD cargo fmt --all -- --check
passed
```

The focused source assertion is in
`crates/dav-server/tests/matrix.rs` as
`date_precondition_matrix_covers_head_delete_report_and_clock_skew`.

## Findings and limits

This receipt proves deterministic local behavior only. It does not claim
clock synchronization, external-client interoperability, distributed
precondition semantics, or remote Anytype date metadata.
