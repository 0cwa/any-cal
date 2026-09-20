# Credential-safety workspace audit

Date: 2026-09-20  
Scope: `/var/home/x/Dev/any-cal` only

## Disposition

This was a read-only workspace scan. It did not access external credential
stores, environment variables, host keyrings, personal Flatpak state, live
services, devices, or Anytype accounts. The scan did not print, copy, save, or
return any secret value or complete identifier.

The scan enumerated 362 regular files outside `.git`, build/dependency output
directories, and editor state. 357 were decoded as text. Four files contained
NUL bytes; three large binary artifacts were deliberately not decoded. Pattern
findings were retained only as path, line number, pattern class, and
disposition.

## Actionable workspace finding

An untracked private workspace artifact contains credential-shaped assignments:

| Location | Pattern class | Disposition |
| --- | --- | --- |
| `.local/anytype-test-v2/credentials.env:20` | opaque API-key-shaped assignment | **actionable secret-bearing artifact; do not publish or reuse** |
| `.local/anytype-test-v2/credentials.env:13,19` | opaque generated name/identifier-shaped assignments | review with the same artifact boundary |

The file is mode-restricted at the directory level. The workspace's `.git`
directory has no usable repository metadata, so an ignore/tracking decision
cannot be established; treat the path as an untracked secret-bearing artifact.
This audit intentionally did not alter or remove it. A separate, explicitly
authorized cleanup should quarantine or securely remove the artifact and add
an ignore rule; until then, the strict no-secret-artifact gate remains open.

## Non-actionable/documentary findings

- `tools/anytype-probe/fixtures/api-key-list-row.txt:3` contains a
  credential-shaped fixture value used to exercise the redacted parser. It is
  classified as synthetic test material, not a live credential, but should
  remain clearly labelled and never be used for authentication.
- `.local/anytype-test-v2/metadata.txt:10` is a 64-character digest-shaped
  metadata field. It is a fingerprint/checksum, not an authentication value.
- Long hexadecimal values in `Cargo.lock`, probe lockfiles, APK/build receipts,
  and safe-change receipts are dependency checksums, artifact digests, or
  redacted fingerprints. They are documentary values, not credentials.
- Authorization/Bearer patterns in Rust source, tests, and planning documents
  are protocol literals or redaction tests. No source line was classified as a
  literal bearer secret.
- Android/provider IDs and Anytype-shaped identifiers in source, tests, and
  receipts are synthetic opaque IDs or documented handles. The scan reports
  their classes only; no complete identifier is reproduced here.

## Historical transient-output incident

The historical CLI provisioning incident is documented in
`docs/plan/anytype-cli-live-current.md` and
`docs/plan/anytype-cli-wire-acceptance-current.md`. Those documents state that
a disposable API key crossed a transient command-output boundary, that the
credential was revoked/guest state removed during the bounded run, and that the
incident prevents a global claim of strict no-secret-output safety.

This workspace-only scan found no retained copy of the incident value in the
text artifacts it could safely classify. It cannot prove that the value never
existed in external shell history, host logs, deleted files, VM images, or
other stores because those locations were explicitly out of scope. The
untracked `.local/anytype-test-v2/credentials.env` finding is a separate
current secret-bearing artifact and keeps the overall disposition **open**.

## Limitations and reopen conditions

Reopen this audit after the credential-bearing workspace artifact is handled,
or before any repository export/release. Repeat the scan after cleanup and
verify that no credential-shaped assignment remains. Separately obtain an
authorized host/guest-log audit if proof about historical transient output
outside this workspace is required. Do not infer live account revocation or
external-store safety from this document.

## Evidence boundary

The authoritative evidence is this document plus the current work-unit receipt
and ledger entry. The scan produced no secret-bearing report artifact. Product
code and external state were unchanged.
