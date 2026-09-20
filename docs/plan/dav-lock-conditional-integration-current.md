# DAV lock/conditional integration receipt

Date: 2026-09-20
Scope: provider-free synthetic `DavServer` and `MemoryRepository` only.

## Result

The local DAV profile does not implement durable WebDAV locks. `OPTIONS` and
error `Allow` headers advertise only `OPTIONS, PROPFIND, REPORT, GET, HEAD,
PUT, DELETE`; `LOCK` and `UNLOCK` receive `405 Method Not Allowed`. The
profile now also rejects a `PUT` or `DELETE` carrying an `If` header with
`412 Precondition Failed` before consulting or mutating the repository. This
fail-closed boundary prevents an invalid, expired, or unsupported lock token
from being silently ignored.

Synthetic coverage verifies:

- valid-looking, expired-looking, and invalid-looking opaque lock tokens all
  produce bounded `412` responses and leave the resource body and ETag intact;
- the same boundary prevents deletion;
- `LOCK` and `UNLOCK` remain explicitly unsupported and do not echo token data;
- a successful ETag-guarded update changes the ETag, while a later stale
  `If-Match` update returns `412` and leaves the newer representation intact;
- rejected paths do not expose request paths, token values, or other secret
  material.

## Verification

Commands, run with `LD_PRELOAD` unset and offline:

```text
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test matrix --offline
43 passed, 0 failed

env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --tests --offline -- -D warnings
passed

env -u LD_PRELOAD cargo fmt --all -- --check
passed
```

## Boundary

This is not evidence of RFC WebDAV lock persistence, refresh/timeout
semantics, external-client interoperability, or distributed concurrent lock
coordination. Implementing those requires a separately designed persistent
lock-token model and client matrix. No network, credentials, Anytype state,
Android state, DAVx5/Tasks.org, personal Flatpak, or pending credential
artifact was accessed.
