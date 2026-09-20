# any-cal app

The app is a headless wiring seam for `DavServer<AnytypeRepository<_>>`.
Configuration uses simple `key=value` files (`--config path`), then environment variables, then
CLI flags. Important names are `ANY_CAL_ANYTYPE_ENDPOINT`,
`ANY_CAL_ANYTYPE_API_VERSION`, `ANY_CAL_SPACE_ID`,
`ANY_CAL_CONTACTS_COLLECTION`, `ANY_CAL_TASKS_COLLECTION`,
`ANY_CAL_LISTEN_ADDRESS`, `ANY_CAL_ANYTYPE_TOKEN`,
`ANY_CAL_TRANSPORT_MODE`, and
`ANY_CAL_MAX_CONNECTIONS`; flags include `--endpoint`, `--api-version`,
`--space-id`, `--listen`, `--token`, `--transport-mode`, and
`--max-connections`.

`check` prints only safe configuration metadata and never prints the token.
`serve` binds the configured local address and uses the live Anytype HTTP
transport by default. The deterministic in-memory transport is available only
with the explicit `--transport-mode fake` (or
`ANY_CAL_TRANSPORT_MODE=fake`) override and is intended for fixtures and local
development. The HTTP transport accepts both `http://` and strict
platform-verified `https://` endpoints; no token is logged by this binary.

`/health` reports local configuration and whether the upstream has been
tested. `/ready` performs a bounded read of the configured Space and returns
`503` until the live transport succeeds (or until required local audit state is
healthy). A configured endpoint is not reported as a reachable Anytype
service merely because the listener started.
The local listener supports bounded concurrent framed requests and explicit
keep-alive/close semantics; it remains suitable for local wiring and protocol
tests, not live synchronization. LAN exposure is fail-closed: loopback is the
default, and `reverse_proxy_tls=true` requires a loopback Any-Cal listener,
`ANY_CAL_AUTH_CREDENTIAL`, and a separately configured HTTPS reverse proxy.
The Any-Cal backend never binds a plaintext non-loopback address in proxy mode.
Set `ANY_CAL_LOCAL_AUTH` for a separate runtime-only credential required by
`/health` and `/status` requests; the global DAV credential remains an
accepted alternate when configured. The local credential is not the Anytype
token, is never accepted for DAV resources, and is not persisted by the GUI.
`ANY_CAL_MAX_CONNECTIONS` bounds active listener connections (default `64`);
excess connections receive `503`.
