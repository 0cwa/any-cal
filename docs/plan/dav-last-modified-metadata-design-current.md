# DAV last-modified metadata — current local design

Status: locally implemented and verified with synthetic resources on 2026-09-20.

## Bounded policy

`StoredResource.modified_at` is a persisted UTC timestamp represented as Unix
seconds. HTTP dates have one-second precision, so retaining sub-second values
would not improve DAV behavior. New writes use the repository wall-clock
second and clamp to `previous + 1 second` when the wall clock is equal or
earlier. This makes rapid writes and backwards clock movement monotonic for a
resource without pretending that a wall-clock synchronization service exists.

The metadata is intentionally outside `ResourceEnvelope.canonical_json()`.
Consequently, `ETag` remains a digest of the same canonical representation as
before; changing only modified-at or archive state does not change the
representation ETag. `StoredResource` is serde-serializable, so a local
repository snapshot/reopen preserves the timestamp and ETag together.

Older serialized `StoredResource` values that lack `modified_at` deserialize
to Unix epoch through `serde(default)`. This is a conservative migration
fallback: the row remains readable, its ETag is not rewritten, and the next
successful mutation assigns a current/clamped value. A future durable
Anytype-side metadata property must preserve this same field and policy; this
lane does not claim that remote Anytype objects already store the sidecar.

## HTTP behavior

- `GET`/`HEAD` emit `Last-Modified` alongside `ETag`.
- `If-None-Match` has precedence over `If-Modified-Since`. A matching ETag
  returns `304`; a non-matching ETag causes the date condition to be ignored.
- Without `If-None-Match`, a valid `If-Modified-Since` date at or after the
  stored second returns `304`.
- `If-Unmodified-Since` guards `PUT` and `DELETE` only when `If-Match` is
  absent. A valid date older than the stored timestamp returns `412` before
  repository mutation. Malformed or out-of-range dates are ignored.
- Successful overwriting `PUT` without an ETag condition is allowed and uses
  the date condition when present. `If-Match` remains the stronger validator.

The parser accepts IMF-fixdate with `GMT`, rejects malformed calendar/time
values and pre-epoch dates, and formats all emitted dates in UTC.

## Evidence

- `cargo test -p any-cal-core -p any-cal-dav-server`: all focused core/DAV
  tests pass (including 33 DAV matrix tests and the new date-precondition
  cases).
- `cargo clippy -p any-cal-core -p any-cal-dav-server --all-targets -- -D warnings` passes.
- `cargo fmt --all -- --check` passes.
- Core tests cover serde snapshot/reopen stability, ETag independence,
  backwards-clock clamping, and archive metadata changes.
- DAV tests cover second-precision `304`, ETag precedence, stale
  `If-Unmodified-Since` no-mutation behavior, malformed-date ignore behavior,
  and successful clamped update.

No network, external client, Anytype credential, Android/provider state,
personal Flatpak, or pending credential artifact was accessed.
