# Repository component-envelope persistence — current receipt

Date: 2026-09-20
Scope: local/offline synthetic repositories only.

## Result

The repository envelope now carries an optional parsed `opaque_calendar`
component tree in `CanonicalDocument`. The editable `StructuredDocument`
projection is still populated from the first `VTODO`; the opaque tree remains
the authoritative DAV representation for calendar reads and reports. This
preserves VCALENDAR child order, nested VALARM/VTIMEZONE components, sibling
VEVENT/VTODO components, repeated properties, parameters, and unknown
extensions without exposing every component as an editable Anytype field.

The DAV PUT path parses the complete calendar tree, extracts the first VTODO
projection, and persists both. GET and REPORT use the persisted tree for task
resources. Existing envelopes without the optional tree retain the prior
projection-based serialization fallback.

## Evidence

- `crates/dav-server/tests/matrix.rs`
  - `mixed_calendar_components_survive_repository_envelope_and_report`
    creates a synthetic calendar containing VTIMEZONE/STANDARD, VEVENT,
    VALARM, VTODO, and unknown properties; verifies ordered tree persistence,
    canonical-envelope-derived ETag, GET preservation, REPORT multistatus
    calendar-data, and archived-resource retention after DELETE.
  - `tasks_collection_advertises_vtodo_without_claiming_vevent_support`
    verifies the task collection advertises VTODO and does not claim VEVENT
    support.
  - Existing matrix coverage verifies hrefs, content types, conditional
    writes, malformed REPORT rejection, and per-resource 404 multistatus
    behavior.
- `crates/core/src/ical.rs` derives serde for the ordered component tree.
- `crates/core/src/envelope.rs` persists the optional tree with backward-
  compatible serde defaults.

Commands (with `LD_PRELOAD` unset):

```text
cargo test -p any-cal-dav-server --test matrix                         22 passed
cargo test -p any-cal-core --tests                                      25 passed
cargo clippy -p any-cal-core -p any-cal-dav-server --all-targets -- -D warnings  passed
cargo test --workspace --exclude any-cal-app --exclude any-cal-ui --lib  all selected lib tests passed
```

No network, external DAV client, Anytype credential, Android/provider state,
personal Flatpak, or pending credential artifact was accessed. This receipt
does not claim interoperability with DAVx5, Tasks.org, or another external
client; those remain separate authority-gated validation work.
