#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
scratch="${ANY_CAL_TMP_ROOT:-$root/tmp}"
case "$scratch" in
  ./*) scratch="$root/${scratch#./}" ;;
esac

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

"$root/scripts/security/check-public-tree.sh"

umask 077
report_dir="$scratch/gitleaks"
mkdir -p "$report_dir"
chmod 700 "$report_dir"

index_tree="$report_dir/index-tree"
mkdir -p "$index_tree"
trap 'rm -rf "$index_tree"' EXIT
git checkout-index --all --prefix="$index_tree/"

scan() {
  local label="$1"
  local mode="$2"
  local source="$3"
  local report="$report_dir/$label.json"
  local stdout_log="$report_dir/$label.stdout.log"
  local stderr_log="$report_dir/$label.stderr.log"
  local status finding_count

  rm -f "$report" "$stdout_log" "$stderr_log"
  set +e
  "$gitleaks_bin" "$mode" "$source" \
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
    printf 'gitleaks scan=%s findings=%s status=clean\n' "$label" "$finding_count"
  elif [[ "$status" -eq 1 ]]; then
    printf 'gitleaks scan=%s findings=%s status=blocked\n' "$label" "$finding_count"
  else
    printf 'gitleaks scan=%s findings=%s status=error code=%s\n' "$label" "$finding_count" "$status"
  fi
  return "$status"
}

overall=0
for scan_spec in \
  'history git ROOT' \
  'index dir INDEX' \
  'worktree dir ROOT'; do
  read -r label mode source_name <<<"$scan_spec"
  case "$source_name" in
    ROOT) source="$root" ;;
    INDEX) source="$index_tree" ;;
    *) printf 'gitleaks scan: internal source error\n' >&2; exit 2 ;;
  esac
  if scan "$label" "$mode" "$source"; then
    status=0
  else
    status=$?
  fi
  if [[ "$status" -gt "$overall" ]]; then
    overall="$status"
  fi
done

# The detailed logs stay mode 0600 under ./tmp for local inspection. They are
# deliberately never printed here, because scanner diagnostics can contain
# source excerpts even when redaction is enabled.
exit "$overall"
