# Android provider projection designs

This directory contains implementation-ready projection contracts and
sanitized examples. It is deliberately separate from the canonical product
plan and contains no provider implementation.

- [android-provider-projections.md](android-provider-projections.md) — envelope,
  Kotlin projection, identity, deletion, conflict, and acceptance contracts.
- [contact-envelope.json](contact-envelope.json) — repeated labelled values,
  group membership, and opaque vCard properties.
- [event-envelope.json](event-envelope.json) — recurrence, attendees,
  reminders, and opaque iCalendar properties.
- [task-envelope.json](task-envelope.json) — VTODO fields and optional
  Tasks.org capability metadata.

All identifiers and values in the examples are synthetic.
