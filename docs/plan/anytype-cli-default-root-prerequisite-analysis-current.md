# Anytype CLI default-root prerequisite analysis

Updated 2026-09-20. This is a read-only analysis of the pinned
`anytype-cli v0.3.6` release. No account, API key, network request, object,
Space, schema, provider, or CRUD operation was performed for this unit.

## Disposition

No additional supported local prerequisite was established that explains the
missing loopback HTTP listener in the default-root service-only runs. The
Anytype sink remains blocked for live adapter and Android end-to-end work.
The correct closure is documentation-only: do not guess an app-link file,
copy private state, alter trust settings, or authorize another account/key
attempt based on this evidence.

## Pinned artifact inspection

The repository artifact is a stripped, statically linked x86-64 ELF reporting
`anytype-cli v0.3.6 (2026-06-17)`. The archive contains one executable. The
read-only command surface confirms:

- `serve --listen-address host:port` defaults to loopback port `31012`;
- `auth create` accepts `--root-path`, `--listen-address`, and
  `--network-config`;
- `auth apikey` commands expose no independent root, config, or endpoint
  option;
- the documented service has internal gRPC/gRPC-Web listeners in addition to
  the HTTP listener.

Filtered binary metadata also contains the expected service/API symbols and
the `DATA_PATH`/XDG-related configuration strings, but a stripped release
does not provide a reliable source-level mapping from those strings to an
additional startup prerequisite. Embedded strings are not treated as a
configuration contract.

## Known supported prerequisites

The following are established by the pinned CLI help, existing source-backed
research, and prior bounded receipts:

1. Start the guest-local `anytype serve` process before `auth create` or API
   key commands. `auth create` against a stopped service explicitly fails.
2. Use a single mode-0700 HOME/XDG environment for the service and every
   `auth` invocation. Do not mix a service root with a different auth root.
3. Do not combine an unverified `DATA_PATH` override with an account
   `--root-path`; the CLI has no documented portable app-link import path.
4. Treat process, transport, and authenticated readiness as separate gates;
   HTTP port readiness alone is insufficient.
5. Use only the pinned binary and a guest-private state root. Do not read or
   export config, keyring, account-directory, or log contents.

These prerequisites were already satisfied in the bounded default-root
service comparisons to the extent they could be checked without secrets.

## What the bounded comparisons establish

The no-network and exact-coordinator-allowlist service-only runs used the
same mode-0700 default-root wrapper, no `DATA_PATH`, no explicit root
override, and no auth command. In both runs the process remained alive but
loopback HTTP port `31012` was not ready within the bounded window. Listener
and process metadata were reduced to structural summaries and cleanup passed.

Therefore the observed absence of `31012` is not explained by the tested
network policy. The receipts do not distinguish an internal service startup
failure from an unobserved local dependency, and they do not justify another
live retry.

## Unsupported assumptions explicitly rejected

- A guessed app-link filename or copied app-link record is not a supported
  prerequisite.
- A different `DATA_PATH`/`--root-path` nesting is only a hypothesis; the
  redacted receipts intentionally do not retain private path values.
- Embedded `strings` output is not proof that an environment variable or
  filesystem layout is supported.
- A live process is not evidence that the HTTP API is ready.
- The Any-Sync coordinator's peer certificates and ports are not an HTTP API
  transport and must not be used to bypass local service readiness.

## Closure and exact reopen condition

Keep the Anytype live sink and Android provider-to-Anytype E2E lanes blocked.
Reopen only if a separately authorized, read-only startup investigation can
produce source-backed or vendor-documented evidence for the missing local
service prerequisite, or a supported HTTPS Anytype gateway is supplied and
verified. Any future live auth/key run must be a new safe-change unit with
fresh disposable state; it must not reuse an unknown-state account or key.

Evidence:

- [`anytype-cli-app-link-default-root-readiness-comparison-current.md`](anytype-cli-app-link-default-root-readiness-comparison-current.md)
- [`safe-change-run-anytype-cli-app-link-default-root-readiness-comparison-20260920.json`](safe-change-run-anytype-cli-app-link-default-root-readiness-comparison-20260920.json)
- [`anytype-cli-auth-current.md`](anytype-cli-auth-current.md)
- [`anytype-transport-architecture.md`](anytype-transport-architecture.md)

