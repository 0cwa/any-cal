# Release security checks

Run the local secret scan from the repository root:

```sh
GITLEAKS_BIN=/path/to/gitleaks scripts/security/gitleaks-scan.sh
```

The helper first rejects private or generated paths in the Git index and
reachable history, then scans history, the staged index tree, and the public
working tree. It writes redacted JSON reports and scanner logs below
`./tmp/gitleaks/` with mode `0600`. It prints only scan counts and status.
`./tmp/` is ignored by `.gitignore`; the local `.git/info/exclude` entry is
retained for existing clones as an additional workspace safeguard.

`check-public-tree.sh` is also run directly in CI before the gitleaks action.
This keeps the path guard independent of the gitleaks allowlist, so a forced
addition of private state or generated output cannot be hidden by an exclusion
rule.

The workflow in `.github/workflows/gitleaks.yml` scans the complete Git history on pushes, pull requests, manual runs, and a weekly schedule. It disables comments, summaries, and uploaded finding artifacts so secret matches do not get copied into secondary GitHub surfaces.

`gitleaks.toml` extends the upstream rules. Its path exclusions cover only private local state and generated build or scratch output. Source, tests, fixtures, documentation, and configuration remain covered by the default rules.
