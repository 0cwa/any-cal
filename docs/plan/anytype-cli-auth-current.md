# Anytype CLI v0.3.6 authentication and app-link revocation

Updated 2026-09-20. This is a read-only research receipt for the pinned
Linux binary. It deliberately does not create an account, API key, session,
Space, or network request, and it does not inspect any existing credential or
Anytype profile.

## Result

The pinned CLI has a guest-only account/key lifecycle, but the missing
app-link error cannot be repaired by inventing a file or by calling a public
remote revocation endpoint.

`auth apikey revoke <id>` is a local CLI operation. The v0.3.6 core API calls
the embedded Anytype service to revoke an app by its nonsecret app ID; the
published core API exposes `CreateAPIKey`, `ListAPIKeys`, and
`RevokeAPIKey(appId)`, but no HTTP/API-key-management endpoint for revocation.
Therefore a safe cleanup receipt requires the same live account-selected CLI
state that can list the app-link records. If that state is absent or the
account root is not the one selected by the embedded service, the key cannot
be safely claimed revoked.

This is a blocker for automated cleanup, not evidence that a key is active.
The correct state is unknown until a fresh, explicitly authorized guest run
can create a new account/key and prove the app ID is listable and revocable.

## Pinned binary evidence

The binary is the repository-pinned `anytype-cli v0.3.6` release (build date
2026-06-17). Its SHA-256 fingerprint was checked locally; only a short,
non-secret prefix is retained in execution notes. The exact command used was:

```text
file <workspace>/.local/anytype-test-v2/cli/anytype
sha256sum <workspace>/.local/anytype-test-v2/cli/anytype | cut -c1-20
<workspace>/.local/anytype-test-v2/cli/anytype --no-update-check version
```

The version command reported `anytype-cli v0.3.6` and the release URL. No
credential-bearing output was produced.

## CLI command contract

Read-only help inspection showed:

```text
anytype auth create <name> [--root-path DIR] [--listen-address HOST:PORT]
anytype auth login [--account-key STRING] [--path DIR]
anytype auth logout
anytype auth apikey create <name>
anytype auth apikey list
anytype auth apikey revoke <id>
```

The API-key commands have no root-path, account-path, config-path, or remote
endpoint option. They operate through the running local CLI service and the
currently selected account. `revoke` takes an app ID, not the API-key secret.
The create command prints the new key, so its stdout/stderr must remain inside
the guest until it is consumed through a private descriptor or stdin.

The CLI has no account-delete command. Logout clears local credentials and
asks the local service to stop the account; it is not proof that a previously
created API key was revoked.

## Private state and path contract

Static source inspection of the v0.3.6 `core/config` package establishes the
following Linux defaults:

| State | Default | Notes |
|---|---|---|
| CLI config | `~/.anytype/config.json` | Mode `0600` when written; may contain `accountKey` and `sessionToken` if the system keyring is unavailable. |
| CLI work directory | `~/.config/anytype` | The embedded service's normal work directory. |
| Account data root | `~/.config/anytype/data` | Overridden by the `DATA_PATH` environment variable; `auth create --root-path` also supplies an account store path. |
| Logs | `~/.anytype/logs` | Must not be copied into host-visible receipts without bounded secret-aware capture. |
| Keyring | OS keyring when available | The CLI reports keyring use separately; otherwise it falls back to the config file. |

The account app-link records are service-managed state associated with the
selected account. The public v0.3.6 CLI help and config package do not define a
portable app-link filename or a supported standalone app-link import flag.
The embedded service symbols include app-link read/list/revoke operations, but
the stripped release does not provide a safe static file-name contract. The
prior live run's “account app-link file missing” error therefore cannot be
resolved by placing a synthetic file at a guessed path.

An empty isolated-root check confirmed that setting `HOME`,
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, and
`XDG_RUNTIME_DIR` to fresh mode-0700 directories does not create usable auth
state. `auth status` only reported that the server was not running, and
`auth logout` reported “not logged in”. This was a no-account, no-network
check; no existing state was read.

## What the earlier failure means

The earlier disposable run proved that account creation, authentication, and
API-key creation can reach the isolated service, but its cleanup command could
not find the app-link record in the chosen isolated root layout. The run must
not be upgraded to “revocation verified”. Logout, deleting guest-local state,
stopping the service, and removing the VM are local cleanup actions only.

The separate later receipt that reports successful key-list disappearance is
not sufficient to repair this unit unless it contains a fresh, non-secret
post-revoke authentication/listing receipt tied to the same app ID. No such
receipt is present in this read-only research result.

## Safe future experiment

Only after a new safe-change artifact grants account/key creation and remote
effects, run the following entirely inside a disposable guest. Do not use the
personal Flatpak, host keyring, host home directory, or an old account.

1. Prepare private guest directories (mode `0700`) for `HOME`, the CLI work
   directory, and a new account root. Start the pinned binary on guest
   loopback only.
2. Run `auth create <unique-name> --root-path <guest-account-root>` while
   redirecting stdout and stderr to a guest-private descriptor. Parse only the
   account ID/key handoff inside the guest; never print the account key.
3. Create one uniquely named API key. Capture the returned key and app ID in
   guest memory/private storage only. The app ID is a handle, not a secret;
   still redact it from general logs.
4. Before any object write, run `auth apikey list` through a structured,
   secret-aware guest capture. Record only count, names, and a redacted app-ID
   fingerprint. Do not inspect `config.json`, keyring contents, account data,
   or logs.
5. Revoke exactly that app ID with `auth apikey revoke <id>`. Treat an
   ambiguous transport failure as **unknown**, never as success.
6. Prove revocation in the guest by listing keys and by attempting a bounded
   authenticated read with the disposable API key. The expected result is
   zero matching app rows and an authentication failure. Preserve only the
   redacted status/count receipt.
7. Logout, stop the service, destroy guest state, stop/remove the VM, and
   verify absence. If any step cannot identify the app-link record, stop and
   retain the explicit “revocation unverified” status.

The existing Any-Cal probe's private app-link prerequisite should therefore be
treated as a capability receipt, not as an app-link secret. It may be marked
`revoke=available` only after steps 4–6 pass in the same fresh guest run. A
synthetic placeholder or a file copied from another account is invalid.

## Current decision

The exact portable app-link file prerequisite is **not established** by the
pinned CLI's public contract. The only supported revocation path is the
account-selected local CLI/gRPC operation. A fresh guest run now proves that
the supported account-root layout is usable when the guest-local `anytype
serve` process is started before `auth create`; no guessed app-link file was
created or inspected.

## Guest experiment receipt — 2026-09-20

The reopened experiment ran in a fresh sparse Fedora 42 microsandbox with the
pinned `anytype-cli v0.3.6` binary mounted read-only. Only exact Anytype
coordinator names on TCP 443/1443 were allowed. No personal Flatpak/profile,
host keyring, old credential, Space, object, schema, or object write was used.

All account/API-key command output was redirected to mode-0700 guest-private
captures. The account key and API key were parsed and consumed inside the
guest; only fixed statuses and redacted failure text crossed the boundary.
The hardened `tools/anytype-probe` capture policy was the governing boundary;
no credential-bearing capture was retained.

Observed sequence:

- `auth create` against a stopped service failed with the exact redacted
  message `Failed to create account: anytype is not running. Start it with:
  anytype serve`. The retry started `anytype serve` on guest loopback first.
- Fresh account creation and API-key creation both returned success.
- Pre-revoke `auth apikey list` contained one matching row, and a bounded
  authenticated `GET /v1/spaces` returned HTTP 200.
- `auth apikey revoke <new-app-id>` returned success; the matching row
  disappeared from the immediate key listing.
- The already-running service still returned HTTP 200 immediately after
  revocation. This is an in-memory authentication-cache observation, not a
  revocation failure and not sufficient proof by itself.
- After restarting the guest-local service, the matching key-list row was
  still absent and the same bounded authenticated request returned HTTP 401.
- `auth logout` returned success; the guest state root was removed and
  verified absent; all nine disposable sandboxes created during the run were
  removed. No account deletion is claimed because the CLI exposes no such
  operation.

This proves the app-ID revoke path and establishes `revoke=available` for a
fresh, account-selected CLI run. Revocation verification must restart or
otherwise invalidate the local service's credential cache before asserting
that an old API key is rejected. The result does not establish a portable
app-link filename or justify inspecting account/config/log contents.

The corresponding bounded safe-change receipt is
`docs/plan/safe-change-run-anytype-cli-app-link-revocation-20260920.json`.

## Layout comparison with the v4 wire retry

The two receipts establish one concrete difference, but not a complete path
map:

| Item | Successful guest revocation run | Failed v4 wire retry |
|---|---|---|
| Service ordering | The first `auth create` was retried only after a guest-local `anytype serve` was started. | The retry also reached authenticated pre-revoke access, but the receipt does not record service-start ordering. |
| Account root | A fresh account root selected by the CLI flow was usable. The actual path is intentionally not retained. | The run used a fresh account and an explicitly shared `DATA_PATH`; the actual path is intentionally not retained. |
| XDG/HOME values | Not recorded in the redacted receipt. | Not recorded in the redacted receipt. |
| App-link result | Revoke succeeded, and post-restart listing/authentication proved the new app link was gone. | Revoke failed with `app link file not found in the account directory`, before any object or schema write. |

Thus the exact observed difference is **the v4 run's explicit shared
`DATA_PATH` versus no recorded `DATA_PATH` in the successful run**, together
with the successful run's explicitly recorded service-before-create retry.
The artifacts do **not** prove that `DATA_PATH` alone caused the failure. They
do not retain the following evidence needed to claim an exact directory
nesting mismatch: the successful run's `HOME`, `XDG_CONFIG_HOME`,
`XDG_DATA_HOME`, `XDG_STATE_HOME`, service `DATA_PATH`, `auth create
--root-path` value, the v4 `--root-path` value, or the effective environment
of both the service and each `auth` command. No app-link filename or account
directory contents may be inferred from these omissions.

The safest interpretation is that v4 mixed or failed to reproduce the
account-root selection context, not that an app-link file should be copied or
invented. A future run must capture only redacted path *identity* metadata
(for example, normalized path labels and equality checks), never path
contents, config, logs, or credentials.

## Corrected no-write provisioning recipe

This is a recipe for a future disposable run; it was not executed here and
creates no account, key, Space, object, or network request.

1. Create one fresh guest root with mode `0700`, then derive labelled private
   children for `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
   `XDG_STATE_HOME`, `XDG_RUNTIME_DIR`, and `ACCOUNT_ROOT`. Do not use the
   host home, keyring, personal Flatpak, or an old root.
2. Launch both `anytype serve` and every `anytype auth ...` invocation through
   the same environment wrapper. Record only whether the labelled values are
   equal or distinct and their modes; do not record their contents.
3. On the first reproduction attempt, **do not export `DATA_PATH`**. Use
   `auth create <unique-name> --root-path "$ACCOUNT_ROOT"` so the explicit
   account-root contract is exercised once, while the service runs first on
   guest loopback. This mirrors the successful receipt's known ordering and
   avoids the unverified v4 `DATA_PATH` override.
4. If a later experiment must use `DATA_PATH`, use a separate fresh guest and
   set it before starting both the service and the CLI, to the exact same
   labelled account-root target. Do not combine a parent `DATA_PATH` with a
   child `--root-path`, and do not change either value between create, list,
   revoke, restart, and post-restart verification.
5. Keep all account/key output on a guest-private descriptor. No shell
   redaction, config inspection, log inspection, app-link file inspection, or
   host-visible credential handoff is allowed. The only accepted app-link
   evidence is successful CLI list/revoke behavior for the newly created app
   ID.
6. Before any object-write stage, require the redacted receipt to show: the
   same effective environment for service and CLI, one account-root selection
   strategy, pre-revoke list/authentication success, revoke success, service
   restart under the same environment, post-restart list absence, and HTTP
   `401` for the old key. If any path identity is unequal or unavailable,
   stop with `revocation=unknown` and do not proceed to live CRUD.

This recipe intentionally leaves the v4 root nesting unresolved: the current
artifacts are insufficient to distinguish “`DATA_PATH` pointed at the wrong
level” from “the service and CLI inherited different values.” That distinction
requires a future no-write preflight receipt containing redacted effective
environment/path-label equality, not filesystem or credential inspection.

## Latest default-root experiment — failed closed before revocation

On 2026-09-20, a new sparse Fedora 42 guest was created with the pinned
`anytype-cli v0.3.6` binary mounted read-only. The guest used one mode-0700
environment wrapper for the service and every CLI invocation. The service was
started on guest loopback before `auth create`; no `DATA_PATH` was exported and
no explicit `--root-path` was supplied. Only fixed status/path metadata was
allowed to cross the guest boundary.

The account-create command succeeded, and the API-key-create command returned
success, but the guest-only parser did not recognize a key and app-ID handoff.
The run therefore stopped before API-key listing, app-link lookup, revoke,
restart, or the post-restart authentication check. This is a parser-gate
failure, not an app-link failure and not evidence of revocation. No Anytype
object, Space, schema, or broad data write was attempted.

Guest-local logout succeeded and the disposable guest was removed and verified
absent. The account/key remote cleanup state is **unknown** because no app ID
was available for a safe revoke operation; the disposable state must not be
reused. No credential-bearing output, raw capture, config, app-link file,
keyring, or log was inspected or copied to the host.

The bounded receipt is
`docs/plan/safe-change-run-anytype-cli-app-link-default-root-20260920.json`.
The next permitted step is parser repair against a redacted structural fixture
or a supported machine-readable handoff, followed by a new explicitly
approved fresh-guest run. Do not infer default-root app-link compatibility from
this attempt.

The parser repair is now covered offline in `tools/anytype-probe`: the pinned
v0.3.6 create shape (`API key created successfully`, `Name:`, `Key:`) yields
field-presence only, while the actual `auth apikey list` table (`NAME`, `ID`,
`KEY`, `CREATED`) yields only an opaque ID fingerprint. The list parser accepts
one row and only the CLI's shortened `8-chars...` key display; duplicate rows,
malformed timestamps, unexpected columns, and unshortened/full key values fail
closed. No live retry has been run.

The table shape is from the pinned source implementation at
`https://raw.githubusercontent.com/anyproto/anytype-cli/v0.3.6/cmd/auth/apikey/list/list.go`:
the command writes the four-column header, sorts by creation time, and shortens
long keys to eight characters plus `...`. This source inspection made the
previous `App ID:` line fixture obsolete; no live command was rerun.

## Reopened lifecycle — app-ID list ambiguity

A second fresh guest lifecycle was authorized after the structural API-key
handoff parser was repaired. It again used service-first startup, one shared
mode-0700 HOME/XDG wrapper, no `DATA_PATH`, and no explicit root override.
The API-key create handoff passed inside the guest-private parser. The first
API-key list output did not match the approved narrow `App ID:`/`ID:` plus
optional `Name:` grammar, so the run stopped before accepting a pre-revoke
identity or attempting revoke. Service restart and post-restart authentication
were not attempted.

The fail-closed cleanup trap attempted guest-local logout, and the fresh guest
was removed. No object, Space, schema, or broad data write occurred; no raw
capture or credential crossed the guest boundary. The remote disposable
account/key state remains **unknown** and must not be reused. This is list
format ambiguity, not an observed app-link failure and not evidence of
revocation.

The retry is recorded in
`docs/plan/safe-change-run-anytype-cli-app-link-default-root-20260920.json`.
The next step is to repair the app-ID list parser from a redacted structural
fixture or supported machine-readable handoff before another live run.

## Strict table-parser retry — app-ID ambiguity

A third fresh guest lifecycle used a strict table-form parser after the
key-create handoff passed. The parser required a recognized ID/name header,
separator, exactly one row matching the newly created disposable key name, and
the exact documented empty marker after revoke. The private list output did
not satisfy that grammar, so the run stopped before accepting the app ID or
attempting revoke. No restart, post-restart authentication, or data operation
was attempted.

The guest cleanup trap attempted logout and the VM was removed. No credential
or raw capture crossed the guest boundary; the remote disposable account/key
state remains **unknown** and must not be reused. This remains an output-shape
ambiguity, not an observed app-link failure or revocation result.

## Final parser retry — stopped pending direct guest capture

The final one-time retry used a fresh pinned-CLI guest, service-first startup,
one shared default-root HOME/XDG wrapper, no `DATA_PATH`, and no root override.
The exact source-backed empty/table parser was used privately: the expected
forms were `No API keys found.` or the `NAME ID KEY CREATED` table with its
shortened key and ISO timestamp. The private list output still failed the
parser gate. The run stopped before accepting the app ID, revocation, service
restart, or post-restart `401` check.

The cleanup trap attempted logout and the guest was removed. No object, Space,
schema, or broad data write was attempted; no credential or raw output crossed
the guest boundary. The remote disposable account/key state remains unknown
and must not be reused. Per the final retry gate, live retries are now stopped
pending direct guest-output capture or a separately authorized parser
correction.

## Sources

- [Anytype CLI v0.3.6 core authentication source](https://github.com/anyproto/anytype-cli/blob/v0.3.6/core/auth.go) — account root, account selection, credential persistence, logout behavior.
- [Anytype CLI v0.3.6 config constants](https://raw.githubusercontent.com/anyproto/anytype-cli/refs/tags/v0.3.6/core/config/constants.go) — Linux work/config/data/log defaults and `DATA_PATH` override.
- [Anytype CLI v0.3.6 config manager](https://raw.githubusercontent.com/anyproto/anytype-cli/refs/tags/v0.3.6/core/config/config.go) — config fields and file mode `0600`.
- [Anytype CLI core package API](https://pkg.go.dev/github.com/anyproto/anytype-cli/core) — `CreateAPIKey`, `ListAPIKeys`, and `RevokeAPIKey(appId)` contracts.
- [Anytype CLI README](https://github.com/anyproto/anytype-cli) — documented command sequence and local HTTP service boundary.
