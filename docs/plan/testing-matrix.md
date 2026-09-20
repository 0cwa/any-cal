# Any-Cal bounded testing matrix

This matrix records reproducible local evidence without converting fixture
coverage into a live-client or live-Anytype claim.

| Area | Exact command/evidence | Result | Classification |
| --- | --- | --- | --- |
| Formatting | `env -u LD_PRELOAD cargo fmt --all -- --check` | Passed | Complete |
| Workspace tests | `env -u LD_PRELOAD cargo test --workspace --offline` | Passed | Complete |
| Lints | `env -u LD_PRELOAD cargo clippy --workspace --all-targets --offline -- -D warnings` | Passed | Complete |
| Android-shaped replay | `env -u LD_PRELOAD cargo test -p any-cal-dav-server --test android_profile --offline` | 2 tests passed | Sanitized fixture evidence |
| Packaging | `env -u LD_PRELOAD bash scripts/test-packaging.sh` | Passed build, checksums, install/uninstall, preservation, unsafe-stage refusal | Complete bounded profile |
| Binary check | `ANY_CAL_SPACE_ID=packaging-smoke target/release/any-cal-app check` | Safe metadata; token not configured | Complete bounded profile |
| Service startup | `ANY_CAL_SPACE_ID=packaging-smoke ANY_CAL_LISTEN_ADDRESS=127.0.0.1:38123 env -u LD_PRELOAD timeout 1 target/release/any-cal-app serve` | Started until bounded timeout; cleanly interrupted | Local-only evidence |
| Health | `curl --max-time 1 -i http://127.0.0.1:38123/health` | HTTP 200 safe JSON | Local-only evidence |

Intentional gaps are live Anytype Space CRUD/envelope/change detection,
live DAVx⁵/Tasks.org/Android device runs, production HTTPS/TLS deployment,
trusted-proxy enforcement, full session authentication, per-client rate
limiting, native platform bundles/signing, and external telemetry. Bounded
in-process authentication and keep-alive behavior are implemented and tested;
these production/client gaps require separate authority or platform
infrastructure and do not block the bounded local profile. Local transport
tests cover bearer-header construction and offset/limit pagination; live
Anytype API pagination and authentication interoperability remain unverified.

Live Anytype work requires a disposable bot account joined to an isolated
Space, an API key, pinned CLI/server versions, endpoint details, redaction
procedure, and explicit external-write authority. Live Android work requires
an authorized emulator/device and disposable DAVx⁵/Tasks.org account.
