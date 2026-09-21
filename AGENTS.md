# AGENTS.md

This repository is primarily developed by coding agents. Optimize changes for the next
agent: keep scope explicit, preserve invariants, leave the tree easier to understand,
and record deferred work in GitHub issues instead of TODO-shaped ambiguity.

## Project mission

Any-Cal is an Anytype translation layer for calendar, task, and contact clients.
Anytype Spaces are the durable backend. The repository currently contains a Rust
CardDAV/CalDAV service plus an Android companion/provider foundation.

The project is pre-production. Breaking changes are acceptable when they simplify the
design, remove legacy ambiguity, or improve correctness. Prefer a clean migration over
compatibility scaffolding that no user needs yet.

## Non-negotiable invariants

- Anytype remains the authoritative durable store. Do not add a second persistent sync
  database.
- DAV or device-specific representations must preserve data that cannot be projected
  losslessly into typed Anytype properties.
- Do not advertise protocol capabilities that are not implemented and tested.
- Never commit live credentials, personal data, device dumps, or unsanitized external
  fixtures.
- Keep Space/account/domain identity explicit. Do not let request data silently choose
  or override an upstream Space.
- Security and recovery paths should fail closed and produce bounded, non-secret
  diagnostics.

## Repository map

- `crates/core`: protocol-neutral models, envelopes, repositories, vCard/iCalendar.
- `crates/anytype-adapter`: Anytype transport, mapping, wire contracts.
- `crates/dav-server`: CardDAV/CalDAV request handling and protocol behavior.
- `crates/app`: configuration, service lifecycle, admin/health surfaces, composition.
- `crates/sync`: bounded sync/checkpoint/export persistence helpers.
- `crates/observability`: audit/event/status support.
- `crates/android-bridge`: Rust/JNI bridge used by the Android companion.
- `crates/ui`: optional Slint setup/status client.
- `android`: Android application/provider implementation.
- `fixtures`: sanitized deterministic protocol/API fixtures.
- `scripts`: validation, Android build/probe, and security helpers.
- `packaging`: release staging and user-local install/uninstall scripts.
- `tools/anytype-probe`: standalone Rust workspace for probing Anytype CLI/API
  behavior. It intentionally has its own `Cargo.lock`.
- `docs/plan`: architecture/research plus execution evidence. Treat `*-current.md`
  files as evidence snapshots, not the canonical place to add new design policy.

## Golden validation commands

Use the same locked/offline profile as CI whenever dependencies are already available:

```sh
env -u LD_PRELOAD cargo fmt --all -- --check
env -u LD_PRELOAD cargo test --workspace --offline --locked
env -u LD_PRELOAD cargo clippy --workspace --all-targets --offline --locked -- -D warnings
env -u LD_PRELOAD scripts/test-packaging.sh
```

Android validation is more expensive and should be run when Android/JNI/build changes
are in scope:

```sh
env -u LD_PRELOAD scripts/android-build/build-and-test.sh
```

For the standalone Anytype probe, run Cargo from `tools/anytype-probe`.

## Change discipline

Before editing a subsystem, read its `Cargo.toml`, nearby tests, and any directly
relevant architecture document. Prefer extracting cohesive modules over extending
already-large `lib.rs` files. New behavior should normally arrive with a focused test
or deterministic fixture.

Keep public contracts small. Reuse workspace dependencies. Avoid introducing another
framework or code generator unless it removes more complexity than it adds.

Do not perform live Anytype, DAV client, Android device, or credential-bearing
experiments unless the task explicitly authorizes them. Default to sanitized fixtures
and local/in-process tests.

When a change reveals work that is real but not safe to include, open or update a
GitHub issue with: priority, motivation, affected files, acceptance criteria, and known
dependencies. Do not leave a bare TODO as the only handoff.

## Documentation discipline

Update the closest durable document when behavior or architecture changes. Avoid
creating a new status snapshot for routine implementation work; prefer the pull request
and issue history unless a reproducible validation record has lasting value.

Keep README claims conservative: distinguish deterministic fixture evidence from live
interoperability or production support.

## Pull request handoff

A good PR explains what changed, why the chosen boundary is safe, which commands were
run, what was not validated, and which issues remain. Keep unrelated formatting churn
out of behavioral changes.
