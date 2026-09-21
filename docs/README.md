# Any-Cal documentation

This is the entrypoint for durable project documentation. Read this before searching
`docs/plan/` broadly: that directory contains both canonical design material and a
large body of historical validation snapshots.

## Read first

For most implementation work, use this order:

1. [../README.md](../README.md) — current supported product profile and conservative
   capability claims.
2. [../AGENTS.md](../AGENTS.md) — repository invariants, validation commands, and
   agent handoff rules.
3. [plan/README.md](plan/README.md) — architecture and protocol scope.
4. [plan/schemas.md](plan/schemas.md) — Anytype/DAV object representation.
5. [plan/development-efficiency.md](plan/development-efficiency.md) — thin-slice
   development boundaries.
6. [plan/testing-matrix.md](plan/testing-matrix.md) — validation layers and what each
   test class proves.

Then read only the domain documents relevant to the change.

## Canonical domain references

These documents are maintained as durable design or operational contracts:

- Domain/Space routing contract: [plan/domain-bindings.md](plan/domain-bindings.md)
- Anytype transport: [plan/anytype-transport-architecture.md](plan/anytype-transport-architecture.md)
- Client interoperability: [plan/client-compatibility.md](plan/client-compatibility.md)
- LAN/security boundary: [plan/lan-security.md](plan/lan-security.md)
- Sync durability: [plan/sync-durability.md](plan/sync-durability.md)
- Observability and recovery: [plan/observability-recovery.md](plan/observability-recovery.md)
- Android provider boundary: [plan/android-provider-contract.md](plan/android-provider-contract.md)
- Android research/constraints: [plan/android-research.md](plan/android-research.md)
- Anytype API probing: [plan/anytype-probe.md](plan/anytype-probe.md)

GitHub issues are the canonical place for active implementation priorities and staged
work. A planning document may explain a design, but it should not duplicate a live
issue backlog.

## Evidence versus design

Files matching `docs/plan/*-current.md` are **evidence snapshots** produced by earlier
validation or implementation lanes. They are useful when a change needs the exact
historical observation they record, but they are not automatically current product
policy and should not be the first documents loaded into an agent context.

When code, tests, the root README, and a historical snapshot disagree, investigate the
difference rather than treating the snapshot as authoritative.

New reproducible validation receipts that have lasting value belong under
[docs/evidence/](evidence/). Routine status belongs in the pull request, CI run, or
GitHub issue instead of a new `*-current.md` file.

## Documentation retention rules

- Update a durable design/contract document in place when its contract changes.
- Do not create a new status snapshot for ordinary implementation work.
- Keep product/support claims in the root README conservative and test-backed.
- Preserve historical evidence when it explains a compatibility, security, migration,
  or recovery decision that cannot be reconstructed cheaply.
- Move superseded receipts to `docs/evidence/` when reorganizing them; do not rewrite
  their conclusions while moving them.
- Delete generated/transient receipts when CI artifacts or PR history already preserve
  the useful information.
- Keep links from durable documents to the minimum evidence needed to support a claim.

The long-term cleanup of legacy `*-current.md` snapshots is tracked in GitHub issue
#13 and should be done in reviewable batches rather than one mass rewrite.
