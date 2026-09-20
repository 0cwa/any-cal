# Local packaging install/rollback resilience

Date: 2026-09-20

Scope was limited to disposable, user-owned temporary roots. No external
network, Anytype state, Android state, DAV client, personal Flatpak, or
workspace credential artifact was read, moved, hashed, or exposed.

## Changes

`packaging/install.sh` now:

- requires the stage marker, manifest, and checksum file;
- verifies every staged checksum before changing the install prefix;
- copies managed files into a temporary same-filesystem transaction directory;
- backs up only the four managed package paths during activation; and
- restores those paths if activation fails part-way through.

User configuration, checkpoints, and unrelated files remain outside the
managed path set. The installer still performs no privileged or system-wide
installation.

The packaging smoke test now rejects a tampered staged binary before
activation and uses an assignment-shaped secret scan that does not mistake
documentation text for a credential.

## Evidence

All results below used fresh temporary roots and synthetic state:

| Check | Result |
| --- | --- |
| `bash -n packaging/*.sh scripts/test-packaging.sh` | pass |
| `env -u LD_PRELOAD bash scripts/test-packaging.sh` | pass |
| Initial install with synthetic config/checkpoint/unrelated file | pass; all preserved |
| Upgrade with changed staged documentation | pass; user state preserved |
| Tampered staged binary | rejected by checksum before activation; installed version unchanged |
| Mid-activation failure caused by a synthetic path blocker | pass; previously installed files restored |
| Uninstall | package files removed; synthetic config/checkpoint/unrelated files preserved |
| File modes | binaries `0755`; documentation `0644`; owner remained invoking user |
| Unmarked stage refusal | pass; sentinel preserved |

No claim is made for privileged service managers, signing/publication, native
Windows bundles, or power-loss recovery during an individual filesystem
rename. The existing service/lock recovery boundary remains separate.
