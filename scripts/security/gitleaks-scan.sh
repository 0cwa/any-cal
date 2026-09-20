#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
scratch="${ANY_CAL_TMP_ROOT:-$root/tmp}"

# Keep this helper workspace-local so a scan never fills the shared /tmp
# filesystem and never silently writes a report beside source files.
case "$scratch" in
  "$root/tmp"|"$root/tmp"/*) ;;
  *)
    printf '%s\n' 'ANY_CAL_TMP_ROOT must point below the repository ./tmp directory' >&2
    exit 2
    ;;
esac

gitleaks_bin="${GITLEAKS_BIN:-}"
if [[ -z "$gitleaks_bin" ]]; then
  gitleaks_bin="$(command -v gitleaks || true)"
fi
if [[ -z "$gitleaks_bin" ]]; then
  printf '%s\n' 'gitleaks is required; install it or run the gitleaks GitHub workflow' >&2
  exit 2
fi

umask 077
report_dir="$scratch/gitleaks"
mkdir -p "$report_dir"
chmod 700 "$report_dir"
report="$report_dir/report.json"
stdout_log="$report_dir/stdout.log"
stderr_log="$report_dir/stderr.log"
rm -f "$report" "$stdout_log" "$stderr_log"

set +e
"$gitleaks_bin" dir "$root" \
  --config "$root/gitleaks.toml" \
  --redact \
  --no-banner \
  --exit-code 1 \
  --report-format json \
  --report-path "$report" \
  >"$stdout_log" 2>"$stderr_log"
status=$?
set -e

finding_count="unknown"
if [[ -f "$report" ]]; then
  finding_count="$(python3 - "$report" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        data = json.load(handle)
except (OSError, ValueError):
    print("unknown")
else:
    if isinstance(data, list):
        print(len(data))
    elif isinstance(data, dict):
        findings = data.get("findings", data.get("Findings", []))
        print(len(findings) if isinstance(findings, list) else "unknown")
    else:
        print("unknown")
PY
)"
fi

if [[ "$status" -eq 0 ]]; then
  printf 'gitleaks findings=%s status=clean\n' "$finding_count"
  exit 0
fi

if [[ "$status" -eq 1 ]]; then
  printf 'gitleaks findings=%s status=blocked\n' "$finding_count"
else
  printf 'gitleaks findings=%s status=error code=%s\n' "$finding_count" "$status"
fi

# The detailed logs stay mode 0600 under ./tmp for local inspection. They are
# deliberately never printed here, because scanner diagnostics can contain
# source excerpts even when redaction is enabled.
exit "$status"
