# Anytype CLI local-HTTP validation

Updated 2026-09-20. This receipt covers a fresh, disposable host-KVM
microsandbox run of the pinned `anytype-cli v0.3.6`. It did not use the
personal Flatpak, a personal Anytype profile, an existing account key, or an
Android/DAV account.

## Environment and cleanup

- Sandbox: `any-cal-cli-live-20260920`, Fedora 42, restricted profile, sparse
  storage, 2 vCPUs, 1 GiB effective / 2 GiB maximum memory.
- Only `.local/anytype-test-v2/cli/anytype` was mounted, read-only. No
  credential file or host profile was mounted.
- Egress was default-deny with DNS plus TCP 443/1443 to the three exact
  Anytype coordinator names. The CLI service listened on guest loopback
  `127.0.0.1:31012`; gRPC/proxy listeners were also observed on 31010/31011.
- A fresh disposable bot account, private Space, and API key were created
  inside the guest. The key was revoked, CLI logout succeeded, guest state and
  logs were removed, and the VM was stopped and removed. No credential value,
  full object ID, or raw payload was retained in this report.

Transient-output caveat: the first account-creation command's table-format
output bypassed the intended shell redaction and exposed the disposable
account key in execution output. It was immediately treated as compromised,
was never copied to a host file or report, and was revoked during teardown.
This means the run proves credential lifecycle cleanup but does **not** pass a
strict no-secret-output gate; future runs must capture account creation in a
guest-only file and redact table rows before any host-visible output.
The hardened `tools/anytype-probe` capture path now treats API-key/access-token
table labels as credential fields, rejects full and partial synthetic sentinel
values, and strips Authorization/Bearer markers before report formatting. This
does not retroactively change the prior receipt or prove the separate account
creation workflow is safe; that workflow still requires a fresh isolated run
with revocation and guest teardown evidence.

## Health and transport evidence

| Layer | Result |
|---|---|
| Process | `anytype serve --quiet --listen-address 127.0.0.1:31012` stayed alive and exposed the expected listeners. |
| Unauthenticated HTTP | `GET /v1/spaces` returned `401 Unauthorized`. |
| Authenticated HTTP | The fresh key returned `200 OK`; `Anytype-Version: 2025-11-08` and JSON content type were present. |
| Response framing | `/v1/spaces` used `Content-Length`; object create/list/delete responses used `Transfer-Encoding: chunked`. The existing probe parsed both successfully. |
| Read-only Rust probe | `tools/anytype-probe` completed space, types, objects, and properties reads and emitted redacted bounded summaries/hashes. The final normal object relist was empty. |
| Token handling | The probe accepted the key through guest stdin and did not emit it in its report. No key was copied to the host. |

## Disposable API operations

The documented API request shape was used for one marked `page` object:

- `POST /v1/spaces/{space}/objects`: `201 Created`, response body 3009 bytes.
- `GET` immediately after create: `200 OK`.
- A `PATCH` carrying `type_key: "page"` was rejected by this pinned local
  runtime with `400 Bad Request` and `bad input: failed to update object,
  invalid type key: "page"`. No update success is claimed. The object was
  still readable with its original state.
- `DELETE` on the correctly identified synthetic object: `200 OK`.
- Follow-up `GET`: `200 OK` with `archived=true`.
- Normal `GET /objects?offset=0&limit=100`: `200 OK` with
  `{"data":[],"pagination":{"total":0,...}}`.
- API-key listing after revocation contained zero rows; logout and teardown
  completed.

## Adapter compatibility finding

The live local HTTP endpoint is reachable and correctly framed, but the
current `HttpAnytypeTransport`/`ObjectRecord` wire model is not compatible
with the actual `2025-11-08` response/request schema:

1. `create_object(ObjectRecord)` serialized the internal record shape and got
   `HTTP 400`; the API expects fields such as `name`, `icon`, `body`,
   `type_key`, and typed property entries (`text`, `checkbox`, etc.).
2. `list_objects` returned `TransportError::Malformed` after receiving a
   valid `200` chunked response. The API returns rich object entries without
   the adapter's required `body`/`revision` fields.
3. `get_object` likewise returned `TransportError::Malformed` because the
   API wraps a single result as `{"object": {...}}` and exposes a richer
   object model than `ObjectRecord`.

This is an implementation blocker, not a network or authentication blocker.
The adapter needs an explicit wire DTO layer (request DTOs and response
envelopes) separated from the internal DAV/cache `ObjectRecord`; it must map
Anytype `name`, `type`, `properties`, `snippet/body`, and archive state into
the canonical envelope while retaining unknown fields. The `PATCH` request
must also follow the pinned runtime's accepted update fields and should not
claim type changes until a valid request is confirmed.

## Follow-up required

- Add fixture-backed DTO tests from the redacted live list/get/create shapes.
- Re-run the same one-object live batch after the wire DTO split; require
  adapter list/get/create/archive/relist success before promoting live
  compatibility.
- Verify whether `PATCH` accepts `type_key` only with a type ID/key variant,
  or whether this pinned runtime rejects type updates; test a property/name
  update without `type_key` in a new disposable run.
- Keep the local CLI route separate from any production HTTPS claim. The
  successful local transport does not prove remote coordinator TLS identity
  or Android trust-store behavior.
