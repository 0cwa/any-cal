# Protocol replay

The sanitized profiles in `fixtures/protocol/` are executable request
sequences for a CardDAV contact and a Tasks.org-style VTODO. Each request line
ends with `=> STATUS`; headers are in parentheses and `$ETAG` is replaced by
the most recently captured response ETag. The replay parser executes every
step, normalizes responses, and compares both trace and repository snapshot
across two fresh runs.

The executable deterministic replay is `cargo test -p any-cal-dav-server
--test replay`; it runs entirely against the in-process `MemoryRepository` and
asserts exact statuses, lifecycle ETags, content types, opaque-field
preservation, archive hiding, and repeatability.

Live client compatibility is not claimed by these fixtures. Unsupported
features remain recurrence, alarms, scheduling, sync tokens, VEVENT, auth,
and TLS.
