# Release security checks

Run the local secret scan from the repository root:

```sh
GITLEAKS_BIN=/path/to/gitleaks scripts/security/gitleaks-scan.sh
```

The helper writes its redacted JSON report and scanner logs below `./tmp/gitleaks/` with mode `0600`. It prints only the finding count and status. `./tmp/` is excluded in this clone through `.git/info/exclude`; keep that exclusion when creating a fresh local clone.

The workflow in `.github/workflows/gitleaks.yml` scans the complete Git history on pushes, pull requests, manual runs, and a weekly schedule. It disables comments, summaries, and uploaded finding artifacts so secret matches do not get copied into secondary GitHub surfaces.

`gitleaks.toml` extends the upstream rules. Its path exclusions cover only private local state and generated build or scratch output. Source, tests, fixtures, documentation, and configuration remain covered by the default rules.
