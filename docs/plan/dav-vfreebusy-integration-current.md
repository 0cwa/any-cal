# Local VFREEBUSY integration receipt

Status: verified locally on 2026-09-20. This receipt covers provider-free,
synthetic data only. It does not claim external scheduling delivery or client
interoperability.

## Evidence

- `crates/dav-server/tests/vfreebusy.rs` exercises a bounded `VFREEBUSY`
  projection, overlap merging, half-open window clipping, and empty output.
- A synthetic availability calendar is persisted through `MemoryRepository`,
  cloned/rebuilt, and read back with the same ETag and `ModifiedAt`. The
  opaque calendar round-trips through the iCalendar parser and the
  `VFreeBusy` model.
- The task-only CalDAV profile rejects a `VFREEBUSY` PUT with `400` and leaves
  the repository unchanged. An unsupported `free-busy-query` REPORT also
  returns `400`; the server does not advertise an unsupported scheduling
  endpoint.
- Empty `VTODO` REPORT output is deterministic `207` multistatus, malformed
  REPORT input is `400`, and existing DAV matrix coverage continues to pass.

## Validation

All commands were run with `LD_PRELOAD` unset:

```text
cargo test -p any-cal-dav-server --test vfreebusy --test matrix --quiet
  vfreebusy: 2 passed; matrix: 44 passed
cargo test -p any-cal-core --test vfreebusy --test freebusy --test repository --quiet
  3 + 8 + 4 passed
cargo fmt --all -- --check
cargo clippy -p any-cal-dav-server --tests -- -D warnings
```

No network, credentials, Anytype state, Android/provider state, DAVx5,
Tasks.org, personal Flatpak, or pending credential artifact was accessed.

## Boundary / follow-up

The current product profile exposes task VTODO resources only; it does not
implement a CalDAV scheduling/free-busy endpoint. A future endpoint needs a
separate collection/request contract and external client interoperability
lane. The current tests preserve that boundary instead of silently treating a
VFREEBUSY component as a task.
