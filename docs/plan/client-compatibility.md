# Client compatibility evidence

`fixtures/protocol/` contains sanitized executable request profiles. Request
lines use `METHOD PATH (Header: value; ...) => STATUS`; `$ETAG` captures the
latest response validator. The replay tests execute every fixture step twice
against fresh `MemoryRepository` instances and compare normalized traces and
canonical final state. They cover CardDAV/VTODO discovery, vCard/VTODO
create/edit/complete/delete, GET/REPORT, ETags, archive hiding, vendor fields,
and deterministic failure/retry tests.

This remains transport-neutral in-process evidence; it is not a live client
run and does not claim Tasks.org or device Contacts interoperability.

Deferred claims: live device/client runs, authentication, TLS, keep-alive,
sync tokens, VEVENT, recurrence, alarms, and scheduling. Live tests require a
separately authorized client environment.
