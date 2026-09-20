# Local DAV OPTIONS, capability, and ACL contract

Date: 2026-09-20

This lane used only synthetic `MemoryRepository` fixtures and provider-free
application identity policy. It did not access external clients or network,
Anytype credentials/state, Android/provider state, the personal Flatpak, or
the pending credential artifact.

## Reproduced defects and fixes

- OPTIONS on an unrelated path previously returned a generic DAV capability
  profile, which could make clients infer unsupported DAV behavior. It now
  returns a bounded `404`.
- Collection OPTIONS responses now advertise the matching media type through
  `Accept`, use an explicit XML capability `Content-Type`, and use
  `Cache-Control: no-store`. CardDAV advertises only `addressbook` and
  `text/vcard`; the task collection advertises only `calendar-access` and
  `text/calendar`.
- Identity-enabled application OPTIONS responses now remove `PUT` and
  `DELETE` from `Allow` for principals whose collection/resource ACL or
  credential capability is read-only. Read methods and the collection's DAV
  extension remain advertised only after normal authentication succeeds.

The existing property/report projection contract remains in force: explicit
unknown properties receive bounded `404` propstats, namespaces are normalized,
and CardDAV/CalDAV report sets are disjoint. Tasks continue to advertise only
VTODO support; unsupported VEVENT and scheduling extensions are not claimed.

## Evidence

Focused provider-free tests passed:

```text
cargo test -p any-cal-dav-server --test matrix --test http
41 matrix + 4 HTTP tests passed
cargo test -p any-cal-app --test identity_middleware
9 tests passed
cargo test -p any-cal-app --test app
41 tests passed
cargo clippy -p any-cal-dav-server --tests -p any-cal-app --test identity_middleware -- -D warnings
passed
cargo fmt --all
passed
```

The tests assert deterministic CardDAV/CalDAV DAV and Accept headers, no-store
capability responses, bounded unrelated-path behavior, read-only OPTIONS
method filtering, and preservation of write methods for a synthetic
read/write principal. All Rust commands were run with the workspace allocator
preload removed and a fresh clippy target where needed.

## Boundary

This proves local capability and ACL advertisement semantics only. It does not
claim full RFC ACL, scheduling, external-client, DAVx5, Tasks.org, TLS, or
live Anytype interoperability.
