# Any-Cal

Any-Cal is a bounded, headless DAV translator whose durable database is an
Anytype Space. It exposes Anytype objects to DAV clients; it does not maintain
a second persistent sync database.

## Current profile

- `any-cal` provides CardDAV Contacts and CalDAV `VTODO` task collections.
- `any-cal-gui` is an optional Slint configuration/status client; it does not
  own DAV state or Anytype data.
- The service defaults to the Anytype HTTP transport. Deterministic fake mode
  is an explicit fixture/development override (`--transport-mode fake` or
  `ANY_CAL_TRANSPORT_MODE=fake`), never an implicit production fallback.
- The HTTP adapter has bounded JSON/TLS behavior and scripted/local exchange
  coverage. A live Anytype endpoint, account, and Space schema still require
  deployment-specific validation; local adapter tests are not a production
  compatibility claim.
- Calendar `VEVENT`, recurrence, incremental sync, and broad client
  compatibility remain extension work.

Contacts and tasks are mapped to typed Anytype properties while preserving
opaque/unknown DAV data in the adapter representation. Tasks can carry links
to Anytype project objects without changing the DAV client model.

## Configuration and diagnostics

The app accepts a `key=value` config file, environment overrides, and CLI
flags. Common settings are:

```sh
ANY_CAL_ANYTYPE_ENDPOINT=http://127.0.0.1:31012 \
ANY_CAL_ANYTYPE_API_VERSION=2025-11-08 \
ANY_CAL_SPACE_ID=fixture-space \
ANY_CAL_LISTEN_ADDRESS=127.0.0.1:8080 \
cargo run -p any-cal-app --offline -- check
```

Use `--config`, `--endpoint`, `--api-version`, `--space-id`, `--listen`,
`--token`, `--transport-mode`, and `--max-connections` as needed. `check`
reports safe metadata only; tokens are masked and are never logged. `serve`
starts the local DAV listener. `/health` distinguishes local configuration
from upstream reachability, and `/ready` performs a bounded read against the
configured Space before reporting readiness.

## Build, test, and package

For the normal pre-PR validation profile, use:

```sh
bash scripts/validate.sh
```

Use `--offline` when locked dependencies are already cached, and add `--android`
when Android/JNI/build changes are in scope. The underlying checks are:

```sh
bash scripts/docs/check-structure.sh
scripts/android-probe/validate.sh
env -u LD_PRELOAD cargo fmt --all -- --check
env -u LD_PRELOAD cargo test --workspace --offline --locked
env -u LD_PRELOAD cargo clippy --workspace --all-targets --offline --locked -- -D warnings

mkdir -p ./tmp
packaging/release.sh --stage ./tmp/any-cal-stage
ANY_CAL_PREFIX="$HOME/.local" packaging/install.sh ./tmp/any-cal-stage
packaging/uninstall.sh
```

The `LD_PRELOAD` workaround is required in the current development
environment. Packaging produces a user-local stage for both binaries,
`MANIFEST`, and `SHA256SUMS`; see [packaging/README.md](packaging/README.md).

## Explicit deferrals

The current profile does not claim successful live Anytype Space validation,
an official Anytype HTTP gateway deployment, trusted-proxy enforcement,
full session/account authentication deployment, arbitrary-client
interoperability over persistent keep-alive connections, VEVENT/recurrence,
or live Tasks.org/device-Contacts runs. The release workflow can publish
unsigned desktop archives and an unsigned Android APK from a version tag;
platform-native installers, signing/notarization, and Windows builds remain
deferred. These require deployment authority, target fixtures, or additional
interoperability evidence.

Developer and architecture documentation starts at
[docs/README.md](docs/README.md).

## Android client profile

The supported Android direction is an Android DAV client connecting to the
headless Any-Cal service. DAVx⁵ can synchronize Contacts into the Android
Contacts provider; Tasks.org can use the CalDAV task collection when its
provider is available. Any-Cal is not a DAVx⁵ database or a platform-wide
Android Tasks provider. The Android module in this repository is a buildable
companion/provider foundation with no live Anytype reconciliation or broad
device-compatibility claim; provider permissions and client support still
belong to the chosen Android application.

For a local development listener, use direct collection URLs such as:

```text
http://HOST:8080/carddav/contacts
http://HOST:8080/caldav/tasks
```

Clients that use RFC 6764 discovery may start at:

```text
http://HOST:8080/.well-known/carddav
http://HOST:8080/.well-known/caldav
```

For Android/LAN deployment, `HOST` is the HTTPS reverse proxy address. Keep
the Any-Cal backend bound to loopback; `reverse_proxy_tls=true` rejects a
non-loopback backend bind, so the plaintext backend port is not directly
reachable from the phone. Plain HTTP is for loopback development only.

The deterministic Android profile covers discovery, stable `UID`/resource
`Location`, ETags and conditional writes/deletes, `text/vcard` and
`text/calendar`, UTC/floating/TZID handling, unknown-field preservation, and
queued-write reconnect behavior. Run it with:

```sh
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test android_profile --offline
```

This is sanitized fixture evidence, not a live-device compatibility claim.
Live DAVx⁵, Tasks.org, and Android Contacts runs remain blocked until an
authorized emulator/device and disposable account are available. The Android
APK/native bridge build is covered by the hosted CI toolchain, but installing
and running Any-Cal as an Android service or native/Slint shell remains
experimental and has no support claim until device smoke tests exist.

For a proxy-protected local service, set `ANY_CAL_LOCAL_AUTH` in the GUI
process environment. The GUI sends this runtime-only credential to `/health`;
it is required for `/health` and `/status`, separate from the Anytype API
token, and never written to a config file. It cannot authorize DAV resources.
`ANY_CAL_MAX_CONNECTIONS` bounds simultaneous listener connections (default
`64`); excess connections receive HTTP `503`.
