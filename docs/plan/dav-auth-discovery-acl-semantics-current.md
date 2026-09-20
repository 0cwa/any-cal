# Local DAV authentication, discovery, and ACL semantics

Date: 2026-09-20  
Scope: local synthetic fixtures and offline repository tests only.

## Result

The authentication, discovery, and authorization boundary is now covered by
the app and DAV protocol matrices. No external client, network, Anytype
credential, Android/provider state, personal Flatpak, or pending credential
artifact was accessed.

Authentication uses one configured DAV credential and a separate optional
local health credential. Missing, malformed, wrong, duplicate, unsupported,
and health-only credentials fail closed with the same `401` challenge without
revealing whether a DAV path exists. A health-only credential cannot authorize
DAV. Request-controlled space paths cannot escape the configured space.

The boundary fixes made in this lane were:

- `401` responses now declare `Content-Type: text/plain; charset=utf-8`,
  matching their body.
- Authentication is evaluated before rate limiting, so unauthenticated
  traffic cannot consume the authenticated request budget.
- `429` responses now also declare their plain-text content type.

No credential value appears in response bodies, events, or this receipt.

## Discovery and capability evidence

The DAV matrix confirms that:

- `.well-known/caldav` and `.well-known/carddav` redirect to the supported
  home sets.
- Principal discovery exposes only the configured principal and supported
  home-set links, with validated proxy-origin handling.
- CardDAV and CalDAV collections advertise distinct capability sets: the
  address book advertises `addressbook`/vCard behavior, while the task
  collection advertises `calendar-access`/VTODO behavior.
- Unsupported reports, components, properties, depths, media types, and
  malformed requests fail closed; resources from the other collection are
  not exposed.
- Collection and resource responses use the protocol content types
  `application/xml`, `text/vcard`, and `text/calendar` where applicable.

The implementation intentionally has no independent expiry or per-token
scope model. “Expired” and “insufficient scope” therefore remain represented
by invalid credentials and configured-space/path boundaries rather than being
claimed as separate token capabilities. A future multi-principal/expiring
credential design needs a separate identity-lifecycle lane.

## Verification

Commands were run with `LD_PRELOAD` unset and a fresh target directory:

```text
env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-target \
  cargo test -p any-cal-app --test app -- --nocapture
30 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-target \
  cargo test -p any-cal-dav-server --test matrix --test http --test replay -- --nocapture
35 passed, 0 failed

env -u LD_PRELOAD CARGO_TARGET_DIR=/tmp/any-cal-dav-auth-target \
  cargo clippy -p any-cal-app --all-targets -- -D warnings
pass

env -u LD_PRELOAD cargo fmt --check
pass
```

The three DAV suites comprise 4 HTTP, 28 matrix, and 3 replay tests. The
focused new authorization test proves that unauthenticated requests to both
known and unknown paths receive identical denial responses and do not consume
the one-request authenticated rate budget.

## Boundary and follow-up

This receipt proves local protocol behavior only. It does not claim live
DAVx5, Tasks.org, or other-client interoperability. TLS termination remains
the separately verified reverse-proxy concern. Per-user ACLs, credential
expiry, and multi-account principals require an explicitly scoped identity
model before implementation.
