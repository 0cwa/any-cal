# Local installed-service health integration

Date: 2026-09-20

Scope was limited to disposable user-owned package prefixes and a pre-existing
non-privileged Fedora toolbox used only because the restricted runner denies
loopback socket creation. No privileged service manager, external network,
Anytype credential/data, Android/provider/client state, personal Flatpak, or
pending workspace credential artifact was accessed.

## Matrix

| Check | Result |
| --- | --- |
| Release stage, checksum verification, install | pass; `scripts/test-packaging.sh` passed |
| Installed `serve` startup | pass; installed binary reached loopback listener in toolbox |
| Unauthenticated health challenge | pass; HTTP 401 with auth challenge |
| Authenticated health JSON | pass; HTTP 200, bounded JSON, fake transport, configured-state booleans and counters only |
| Health/status secrecy | pass; no synthetic credential, endpoint identifier, checkpoint path, or token value in response or service output |
| Declared checkpoint path | pass; parent was created, writer lock appeared while serving, and lock was removed after normal shutdown |
| SIGTERM shutdown | pass; process exited normally and lock was released |
| Restart from existing checkpoint | pass; authenticated `/status` returned HTTP 200 |
| Package upgrade | pass; a disposable version-`0.1.1` stage activated over the installed stage |
| Package rollback | pass; reinstalling the prior stage restored the installed package paths |
| Post-rollback restart | pass; authenticated `/status` returned HTTP 200 |
| Safe installed `check` | pass; output contained only endpoint scheme/host/port, API version, configured booleans, and limits; no credential values or checkpoint path |
| Uninstall preservation | pass; package files removed while synthetic checkpoint/config state remained |
| Focused app integration tests | pass; 28 tests |
| Focused clippy | pass; `-D warnings` |
| Formatting | pass in the existing workspace package/release smoke lane; no source changes were needed here |

The first direct runner attempt could not create a loopback socket (`EPERM`).
The same installed binary and lifecycle matrix was then run inside the
existing local toolbox; the toolbox was not used as a persistent application
or service installation.

## Boundaries

This receipt does not claim privileged system-service integration, production
deployment, signing/publication, native bundles, power-loss atomicity, or
external TLS/DAV-client interoperability. No focused installer or service
code fix was reproduced as necessary in this matrix.

All temporary prefixes, service processes, checkpoint locks, logs, and staged
artifacts were removed after validation.
