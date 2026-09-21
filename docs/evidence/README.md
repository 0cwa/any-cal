# Validation evidence

This directory is for durable, reproducible evidence that is useful beyond the pull
request or CI run that produced it.

Use it for results such as:

- protocol interoperability receipts that support a documented compatibility claim;
- security or recovery fault-injection results that justify a durable invariant;
- migration/format observations that future code must preserve;
- environment-specific validation that is expensive or impossible to reconstruct from
  unit tests alone.

Do not use it for routine implementation status, agent scratch notes, work ledgers, or
CI output that is already available as an artifact.

## Naming

Prefer a dated path and descriptive capability name:

```text
docs/evidence/YYYY-MM/<capability>-<environment>.md
```

Evidence documents should state the source revision, environment/fixture boundary,
commands or procedure, observed result, and what the result does **not** prove.

Legacy `docs/plan/*-current.md` files predate this structure. Move them here only in
reviewable batches with link repair; do not change their historical meaning during a
move.
