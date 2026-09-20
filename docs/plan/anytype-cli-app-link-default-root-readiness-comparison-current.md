# Anytype CLI default-root readiness comparison

Updated 2026-09-20. This was an evidence-only comparison of the pinned
`anytype-cli v0.3.6` service under the same mode-0700 HOME/XDG default-root
wrapper, first with no network and then with the exact previously approved
coordinator allowlist. The binary was mounted read-only in fresh Fedora 42
microsandbox guests.

Both cases stopped before authentication or any account, key, app-link,
Space, schema, object, CRUD, provider, or Android operation. The process was
alive after 20 seconds, but loopback HTTP port 31012 was not ready in either
case. Both cases reported two aggregate TCP rows and zero established rows.
The reduced service log had 246 bytes, four lines, maximum line length 83,
zero ANSI markers, and ten control bytes in both cases. Only salted structural
hashes were retained in the JSON receipt.

## Result

The approved coordinator allowlist did not change readiness in this wrapper:
network availability does not explain the observed absence of 31012 within
the bounded window. The cause remains unresolved; local default-root
initialization or another service prerequisite is still possible. This result
does not prove app-link compatibility or Anytype API compatibility.

The comparison therefore does not justify another live account/key lifecycle
attempt. The parser and CRUD gates remain unchanged. A further investigation,
if worthwhile, should be separately scoped to local service prerequisites and
must remain before authentication.

Receipt: [`safe-change-run-anytype-cli-app-link-default-root-readiness-comparison-20260920.json`](safe-change-run-anytype-cli-app-link-default-root-readiness-comparison-20260920.json)

All disposable services and guests were removed and independently checked.
No raw logs, credentials, identifiers, or account state were retained.
