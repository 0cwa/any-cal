# Android–Rust bridge review

**Status:** review only; no bridge or product code changed.

## Disposition

The smallest viable seam is a versioned, coarse-grained DTO boundary with
Rust-owned canonical state and Kotlin-owned Android provider operations. Keep
the existing Kotlin interface as the host hook, but replace the eventual
`syncOnce(capabilities)` payload with explicit request/result DTOs. Do not pass
Rust traits, Android `Cursor`/Binder objects, or arbitrary framework objects
across the boundary.

For the first implementation, use one bounded `sync(request)` call per
WorkManager/sync-adapter run:

```text
BridgeRequest {
  schema_version,
  adapter_kind,
  account_scope,
  provider_capabilities,
  checkpoint,
  provider_changes,
}

BridgeResult {
  schema_version,
  outcome,                 # applied | unchanged | unsupported | retryable | conflict
  next_checkpoint,
  projection_operations,
  tombstones,
  conflicts,
  field_statuses,
  redacted_diagnostics,
}
```

The bridge should not persist a second authoritative checkpoint. Rust's
existing sync store owns observed identities, pending operations, tombstones,
generation, and atomic recovery. Kotlin supplies provider observations and
applies returned provider operations; Rust returns the next checkpoint only
after the canonical state transition is durable.

## Repository evidence

The current Kotlin seam is deliberately a no-op: `RustSyncBridge` exposes only
`syncOnce(ProviderCapabilities)` and `NoOpRustSyncBridge` returns `NotLinked`
([RustSyncBridge.kt](../../android/app/src/main/kotlin/org/anycal/android/RustSyncBridge.kt):3-22).
Capabilities currently contain provider/permission booleans and Tasks.org
package version, but no account scope, adapter version, supported columns,
checkpoint, or source-ID namespace
([ProviderCapabilities.kt](../../android/app/src/main/kotlin/org/anycal/android/ProviderCapabilities.kt):9-52).

Rust already has a versioned `ResourceEnvelope` containing collection/resource
IDs, DAV kind, Anytype object ID, DAV UID, canonical structured document, and
revision ([crates/core/src/envelope.rs](../../crates/core/src/envelope.rs):7-31).
Validation rejects empty identities and unsupported document versions, while
`canonical_json` gives deterministic normalized serialization
([crates/core/src/envelope.rs](../../crates/core/src/envelope.rs):52-90).

The canonical structured document preserves repeated property occurrences and
parameters through `BTreeMap` fields ([crates/core/src/model.rs](../../crates/core/src/model.rs):66-108).
The sync store already tracks observed records, pending operations, tombstones,
ETags/revisions, and generation with atomic backup recovery
([crates/sync/src/lib.rs](../../crates/sync/src/lib.rs):14-93,
[crates/sync/src/lib.rs](../../crates/sync/src/lib.rs):130-205).

The workspace has no UniFFI/JNI/NDK dependency, Android target, generated
bindings, `cdylib` crate, or Android native-library packaging in its current
Cargo/build files. The Android module has only AndroidX Core and WorkManager
dependencies and no Gradle wrapper or local Android SDK in the reviewed
environment ([android/app/build.gradle.kts](../../android/app/build.gradle.kts):1-28,
[android/README.md](../../android/README.md)). This is a foundation/toolchain
gap, not a reason to widen the DTO design.

The current Anytype HTTP transport stores a token and emits an Authorization
header, but accepts only `http://` endpoints
([crates/anytype-adapter/src/lib.rs](../../crates/anytype-adapter/src/lib.rs):286-335).
The Android bridge must not claim secure live Anytype transport until that
constraint is separately resolved and tested.

## DTO contract

### Rust-to-Kotlin envelope

Use a bridge DTO that carries the validated canonical envelope plus a compact
projection view. The canonical JSON is retained as a versioned opaque string
only if the bridge runtime cannot bind the Rust structs directly; Kotlin must
never modify it or treat it as provider truth.

```text
EnvelopeDto {
  schema_version: u8,
  canonical_id: String,
  collection_id: String,
  resource_id: String,
  kind: contact | contact_group | task | event,
  anytype_object_id: String,
  dav_uid: String,
  revision: u64,
  canonical_json: String,
  canonical_hash: String,
}
```

The Android adapter receives typed projection DTOs derived from this envelope,
not a mutable `Map<String, Any?>`. Repeated values remain lists; unknown
properties remain opaque occurrences. Any schema-version mismatch is a
fail-closed `Unsupported` result, never a best-effort field drop.

### Kotlin-to-Rust observations

```text
ProviderObservation {
  adapter_kind,
  account_name,
  account_type,
  collection_source_id,
  provider_row_id,
  provider_source_id,
  normalized_hash,
  modified_marker,
  deleted,
}
```

Provider row IDs are operational. `provider_source_id` maps to
`RawContacts.SOURCE_ID` or CalendarContract `_SYNC_ID`; the canonical DAV UID
and Anytype object ID remain separate. Observations must identify their account
and adapter namespace to prevent DAVx⁵/EteSync/direct-provider loops.

### Operation result

Every operation result must include:

```text
operation_id,
canonical_id,
provider_row_id,
provider_source_id,
observed_hash,
field_statuses: preserved | normalized | unsupported,
result: applied | no_op | conflict | retryable | permission_denied | unsupported,
```

No token, Authorization header, raw contact value, or unredacted provider
payload belongs in diagnostics. Errors may include operation ID, adapter kind,
schema/capability version, and stable hashes only.

## Why this is smaller than direct object exposure

- Rust remains responsible for identity, canonicalization, revisions, pending
  operations, retries, tombstones, and conflicts.
- Kotlin remains responsible for permissions, `ContentResolver`, account
  ownership, provider row IDs, and Android scheduling.
- Calendar/contacts/tasks adapters can evolve their typed projection fields
  without changing the checkpoint or Anytype transport contract.
- A single version handshake detects incompatible native libraries, DTOs, or
  provider capability snapshots before writes.
- JSON/bytes can be a temporary transport representation, but canonical Rust
  validation and an explicit schema version prevent it becoming an unbounded
  cross-language mapping DSL.

## Toolchain and FFI risks

These are repository integration risks, not settled library claims:

1. **Native packaging:** Android ABIs, Rust target triples, NDK version,
   linker settings, and generated-library packaging are not present in the
   current workspace. Pin them before implementation.
2. **Binding generation:** choose one reviewed binding path (UniFFI or JNI),
   pin its generator/runtime versions together, and fail CI when generated
   Kotlin/API artifacts drift.
3. **Panic and exception boundary:** no Rust panic or Kotlin exception may
   cross the native boundary. Convert failures into bounded result enums.
4. **Threading:** invoke Rust from WorkManager/sync-adapter background work,
   never the Android UI thread. Keep `ContentResolver` calls in Kotlin.
5. **Cancellation and bounds:** include request IDs, deadline/cancellation
   semantics, maximum envelope size, and maximum operation count.
6. **Memory ownership:** copy strings/bytes at the boundary or use generated
   ownership rules; never retain pointers to Kotlin-managed memory.
7. **Secret surface:** credentials stay in the Android/Rust secure
   configuration path; DTOs, generated debug `toString`, crash reports, and
   logs must contain only redacted metadata.
8. **Transport security:** the current Rust HTTP transport rejects HTTPS, so
   a native bridge must not silently route production credentials through it.

## Acceptance tests before enabling live projection

### Canonical envelope and DTO

- Serialize/deserialize a contact, task, and event with repeated labelled
  values, opaque properties, and parameters; canonical JSON and hash are
  stable across map insertion order.
- Reject empty IDs, unsupported schema versions, malformed JSON, oversized
  payloads, and unknown required operation versions.
- Verify Kotlin cannot change the canonical envelope through an apply result.

### Checkpoints, retries, and tombstones

- Restart after each checkpoint commit phase and verify the Rust sync store
  restores the primary/backup state without losing pending operations.
- Replay the same operation ID and verify idempotence; reuse an operation ID
  with different content and verify rejection.
- Delete/archive a resource, persist a tombstone, then present it in a later
  observation; verify resurrection requires explicit authorization.
- Change ETag/revision while an operation is pending; verify `Conflict`, not
  implicit overwrite.

### Provider boundary

- Missing authority, denied permission, missing Tasks.org capability, and
  account mismatch return `Unsupported`/`PermissionDenied` with no write.
- Provider row recreation with the same source ID repairs operational IDs;
  missing/ambiguous source IDs stop for review.
- Apply result contains only bounded metadata and no token/contact payload in
  diagnostics.
- ContentObserver echo with the projected hash is a no-op; a differing hash is
  an external edit.

### Native/toolchain gates

- Build and load the native library for every supported ABI on a disposable
  API-35 emulator.
- Run a bridge smoke test through the generated binding, including malformed
  request, cancellation, timeout, panic/exception conversion, and process
  restart.
- Verify generated bindings and Rust/Kotlin schema versions are reproducible
  in CI.

## Unresolved decisions

- UniFFI versus handwritten JNI remains open until the Android NDK/toolchain
  can be provisioned and a tiny DTO smoke build is measured.
- Whether canonical envelopes cross as generated records or validated UTF-8
  JSON is an implementation choice; generated records are preferable for
  typed identity/result fields, while opaque DAV occurrences may remain a
  bounded JSON/string field.
- The Android bridge must decide where the durable Rust sync store lives and
  how its path/key material is provided without exposing credentials to Kotlin
  UI code.
- Secure HTTPS Anytype transport and Android credential storage are separate
  gates; bridge work must not waive them.

## Review conclusion

No bridge code change is justified in this lane. The existing no-op hook is
appropriate until the native build/toolchain and binding choice are authorized.
The versioned DTO approach is the smallest path that preserves the existing
Rust canonical envelope/checkpoint semantics while keeping Android provider
ownership in Kotlin. The next safe action is a disposable native hello-world
binding plus schema/version/redaction tests, followed by one-way calendar
projection—not bidirectional Anytype sync.
