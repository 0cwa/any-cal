# Any-Cal Android companion

The Android companion exposes the Any-Cal account through the platform sync
adapter contracts and projects validated canonical contact and event
envelopes directly into `ContactsContract` and `CalendarContract`. The sync
adapter owns provider IDs and scoped writes; the Rust bridge owns the
canonical Anytype/DAV identity, checkpoint, tombstone, and response policy.

The bridge sends a bounded `POST /android/sync` request to the configured
gateway. The request contains the account scope, checkpoint, tombstones, and
full canonical resource envelopes. The gateway returns decisions plus any
canonical resources to project. The native transport uses rustls platform
verification for HTTPS after binding the verifier to the Android JVM and
application trust store; HTTPS fails closed if that initialization is not
complete. No live trusted HTTPS endpoint is claimed by the current receipts.
Provider state and checkpoints are committed only after a successful provider
batch. A missing endpoint, credential, permission, provider, or response
decision fails the run without treating the source as empty.

There is no Android platform task provider. Tasks.org remains an optional,
runtime-gated adapter and is not enabled by this module unless its documented
authority and disposable CRUD contract are present.

## Toolchain assumptions

- Android API 35 / `compileSdk = 35`
- `minSdk = 26`, `targetSdk = 35`
- JDK 17
- Android Gradle Plugin 8.7.3
- Gradle 8.10.2
- Kotlin 2.0.21
- AndroidX Core KTX 1.15.0 and WorkManager 2.10.0
- Rust 1.98.1 (selected by the repository `rust-toolchain.toml`)
- `cargo-ndk` 4.1.2

Once the pinned toolchain is available, run the repository build harness from
the repository root. A Gradle wrapper is not checked in; the harness builds
the native bridge for both declared ABIs before Gradle packages the APK:

```sh
GRADLE_BIN=gradle \
  scripts/android-build/build-and-test.sh
```

The first executable gate must be a disposable API-35 emulator capability
probe. Any missing Tasks.org authority/version/permission leaves only the
optional task adapter unsupported; Contacts/Calendar must remain independent.
