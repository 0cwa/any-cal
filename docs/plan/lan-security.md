# LAN security boundary

The service is loopback-only by default (`127.0.0.1`/`[::1]`). The
`reverse_proxy_tls` mode is proxy-only: it requires the Any-Cal listener to
remain loopback-bound, so the plaintext backend cannot be reached directly
from the LAN. A LAN-facing HTTPS reverse proxy owns the external address and
forwards to the loopback listener. Startup rejects a non-loopback bind when
`reverse_proxy_tls=true`.

The proxy-backed configuration is:

```ini
allow_lan=false
reverse_proxy_tls=true
auth_credential=...
listen_address=127.0.0.1:8080
```

The application does not terminate TLS. `reverse_proxy_tls` is a contract:
an administrator must put an HTTPS/authenticating reverse proxy in front of
the service, bind the service on loopback, and forward only the single
configured Space. Direct plaintext LAN exposure is refused, including when a
non-loopback address is combined with `reverse_proxy_tls=true`. Native TLS,
certificate management, trusted-proxy source enforcement, and a full
user/session authentication system remain deferred.

The bounded inbound authentication profile accepts `Authorization: Bearer
<credential>` or `Authorization: Basic <credential>` and compares the opaque
credential in constant time. Credentials may be supplied by the config
environment/file boundary, are never included in health output or diagnostics,
and must not be placed in command-line arguments. Health and status use the
same authentication gate as DAV requests. The configured Space is the only
scope; request-controlled Space IDs are not routed.

The GUI has a separate runtime-only health credential via
`ANY_CAL_LOCAL_AUTH`. When configured, this credential is required for
`/health` and `/status`; the global `auth_credential` remains an accepted
alternate when present. The local credential is not an Anytype API token and
is never written by the GUI. The GUI may also receive it through its password
field for the current process, but saved configuration excludes it. The local
credential is never accepted as DAV resource authentication.

A process-wide fixed-window request limiter is enabled by default. It includes
failed authentication attempts and returns `429` after the configured limit.
The listener also bounds active connections with `max_connections` (default
64, configurable through `ANY_CAL_MAX_CONNECTIONS` or `--max-connections`).
Excess connections receive `503`; per-client/IP limiting remains a reverse
proxy responsibility.

For Android/LAN use, connect Android to the reverse proxy's HTTPS address;
do not publish the Any-Cal backend port. Account for Android network
permission, background/Doze, certificate trust, and local-network discovery
behavior. Cleartext HTTP is suitable only for loopback development.

Evidence is limited to in-process app tests and the existing framed listener
tests. No live device, reverse proxy, or personal Anytype service was used.
