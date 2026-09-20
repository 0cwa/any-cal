# Anytype CLI wire-DTO acceptance

Updated 2026-09-20. This lane ran the current Rust `HttpAnytypeTransport`
against a fresh guest-local `anytype-cli v0.3.6` service. It stopped at the
first live create mismatch and makes no CRUD-success claim.

## Outcome

The transport reached the isolated local API and decoded a live list response,
but the first DTO-backed create was rejected by the API:

```text
HTTP 400
{"object":"error","status":400,"code":"bad_request","message":"bad input: could not determine property link value type"}
```

The response is a wire-contract mismatch in the property encoder. The current
DTO emits property entries as `{ "name": ..., "value": ... }`; the pinned
runtime requires a typed property-link value shape for the observed property
keys. No update, archive, or relist-after-write was attempted after this
failure.

## Isolation and transport evidence

- Runtime: fresh sparse Fedora 42 microsandbox, removed after the run.
- Binary: pinned `anytype-cli v0.3.6`, mounted read-only.
- Network: default deny; only the three documented Anytype coordinator names
  on TCP 443/1443 were allowed.
- Service: `127.0.0.1:31012`; no host port, personal Flatpak, personal
  profile, old credential, or old Space was used.
- Rust client: a temporary offline-built binary linked to the workspace
  `any-cal-anytype-adapter` crate and invoked the actual `HttpAnytypeTransport`.
- Authenticated health: `200`; unauthenticated health: `401`.
- Live health/list responses in this run used `Content-Length`; no live
  chunked response was observed. Chunked parsing remains covered by the
  scripted transport tests, not this live receipt.
- Read-only pre-write list decoded successfully and contained zero objects.
- A post-failure read returned `200` with no probe marker.

## Credential boundary and cleanup

The first provisioning attempt was abandoned because an insufficient shell
redactor exposed fragments of the CLI's wrapped account key. The replacement
attempt kept account/API-key creation output guest-private, but a later
diagnostic inspection of the guest config also violated the strict secret-safe
boundary. Both disposable accounts are therefore treated as compromised and
must not be reused. The replacement API-key revoke command could not complete:
the pinned CLI reported that its account app-link file was missing. Guest
logout, guest-state removal, service termination, and microsandbox removal did
complete. The pinned CLI exposes no account-delete command, so remote account
deletion and successful key revocation are not claimed.

This is an acceptance failure, not a successful live run. No credential value,
raw response payload, full Space ID, or object ID is retained in this report.

## Required follow-up

1. Fix the wire property encoder from a redacted live request/response fixture
   and add typed property-link DTO tests for text and checkbox values.
2. Add an account-provisioning harness that never prints account-create output
   or config contents. The hardened probe now fails closed unless a private
   account app-link file and a private `revoke=available` capability receipt
   are present; require that guest-only handoff and a verified revocation path
   before another live run.
3. Repeat the same finite batch only after those gates pass; require create,
   get, update, archive, direct archived read, and normal relist omission.

## Retry — v4 revocation gate

A fresh v4 Fedora 42 guest used a new account, key, Space, explicit shared
`DATA_PATH`, and guest-private capture. Authenticated pre-revoke access
returned `200`, but `auth apikey revoke` failed before any object or schema
write with the pinned CLI error:

```text
Failed to revoke API key: API error: app link file not found in the account directory
```

The run stopped before Rust CRUD, then logged out, removed guest state, and
removed the VM. The separate
`safe-change-run-anytype-cli-app-link-revocation-20260920.json` receipt remains
the only successful revocation proof; this retry does not upgrade the wire
acceptance gate.

## Final corrected-recipe retry — v5

The documented no-write recipe was followed in a fresh guest: one mode-0700
root, labelled private children, one environment wrapper for every service and
CLI invocation, service started first, no `DATA_PATH` export, and
`auth create --root-path` pointed at the labelled account root. Only fixed
metadata crossed the boundary:

```text
pre_revoke_auth=200
revoke_error=app link file not found in the account directory
path_metadata=single_wrapper data_path_exported=false service_cli_wrapper_same=true account_root_distinct=true
```

The run stopped before Rust CRUD, then logged out, removed the guest root, and
removed the VM. This rules out the previously suspected `DATA_PATH` override
and mixed-wrapper setup as sufficient explanations, but it does not establish
the missing app-link state. No further live acceptance retries should be
performed without new evidence about the pinned CLI's account/app-link
selection.
