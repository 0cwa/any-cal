# Local CardDAV addressbook-query/property-filter receipt

Date: 2026-09-20
Scope: offline synthetic `MemoryRepository` contacts only.

## Result

The bounded CardDAV addressbook query matrix passes 28 tests. It now covers:

- UID and repeated labelled TEL values;
- case-insensitive text matching across repeated values;
- requested `getetag`/`address-data` projection and XML escaping;
- deterministic resource ordering by resource ID;
- missing/unknown properties and namespaced report elements;
- malformed and explicitly empty addressbook filters returning `400`.

The implementation fix is narrow: an omitted filter remains an unfiltered
query, while an explicit empty `<filter/>` or `<filter></filter>` is rejected
instead of silently broadening the query to every contact.

## Verification

Commands, with `LD_PRELOAD` explicitly unset:

```text
env -u LD_PRELOAD cargo fmt --check                         # pass
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test matrix -- --nocapture
                                                               # 28 passed, 0 failed
env -u LD_PRELOAD cargo clippy -p any-cal-dav-server --all-targets -- -D warnings
                                                               # pass
```

No external network, DAV client, Anytype state, Android/provider state,
personal Flatpak, credential, or pending credential artifact was accessed.

## Boundary

This receipt proves local protocol behavior only. It does not claim
interoperability with DAVx5, Tasks.org, or other external clients.
