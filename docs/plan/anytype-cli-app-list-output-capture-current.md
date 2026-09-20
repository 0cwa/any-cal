# Anytype CLI API-key list structural capture

Updated 2026-09-20. This receipt records the bounded guest-only capture
required to diagnose the pinned `anytype-cli v0.3.6` app-list parser. It does
not retain raw CLI output, account keys, API keys, app IDs, names, timestamps,
configuration, logs, or account-directory contents.

## Result

The final fresh guest completed service-first account creation, API-key
creation, and `auth apikey list` successfully. The list output was captured
inside the guest and reduced to counts, byte lengths, token lengths/classes,
whitespace layout, control-byte counts, and salted token hashes. The list was
a three-line whitespace table:

- line 1: 4 tokens, lengths `4,2,3,7`
- line 2: 4 tokens, lengths `4,2,3,10`
- line 3: 8 tokens, lengths `7,10,3,1,64,11,10,8`

The row therefore has a four-token display name, then a 64-character ID-shaped
field, an 11-character shortened-key-shaped field, and 10/8-character
timestamp-shaped fields. The result has zero ANSI escape bytes and zero
control bytes. The parser-relevant JSON receipt is
[`safe-change-run-anytype-cli-app-list-output-capture-20260920.json`](safe-change-run-anytype-cli-app-list-output-capture-20260920.json).

## Isolation and cleanup

Three fresh Fedora 42 microsandboxes were used because the first guest's
summarizer itself had a shell syntax error and the second capture was repeated
with token lengths; no raw output crossed the boundary in either attempt.
Every guest used a mode-0700 shared HOME/XDG wrapper, started the service
before auth commands, exported no `DATA_PATH`, and supplied no explicit auth
root path. The capture binary was mounted read-only. No personal profile,
Flatpak, keyring, object, Space, schema, provider, or adapter operation was
used. Guest logout cleanup ran, and all three capture guests were removed.

Revocation is deliberately **unknown** for these fresh disposable keys: the
app-ID value never left the guest, so the bounded lane did not attempt revoke.
The accounts and keys must not be reused. This is not a revocation claim.

## Required next lane

Use the structural shape above to add or correct a redacted parser fixture,
run the offline parser tests, and only then request a separate approval-gated
fresh lifecycle run. Keep the parser fail-closed and continue to reject raw
credential handoff. Do not infer the exact app-ID value or inspect private
account state from this receipt.
