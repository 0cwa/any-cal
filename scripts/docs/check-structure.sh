#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

required=(
  "README.md"
  "AGENTS.md"
  "docs/README.md"
  "docs/plan/README.md"
  "docs/plan/schemas.md"
  "docs/plan/development-efficiency.md"
  "docs/plan/testing-matrix.md"
  "docs/evidence/README.md"
)

for path in "${required[@]}"; do
  if [[ ! -f "$path" ]]; then
    printf 'documentation structure: missing required entrypoint: %s\n' "$path" >&2
    exit 1
  fi
done

baseline_file="docs/evidence/legacy-current-baseline.txt"
baseline="$(tr -d '[:space:]' < "$baseline_file")"
if [[ ! "$baseline" =~ ^[0-9]+$ ]]; then
  printf 'documentation structure: invalid legacy snapshot baseline\n' >&2
  exit 1
fi

current="$(git ls-files 'docs/plan/*-current.md' | wc -l | tr -d '[:space:]')"
if [[ "$current" != "$baseline" ]]; then
  printf 'documentation structure: legacy *-current.md count changed: baseline=%s current=%s\n'     "$baseline" "$current" >&2
  printf '%s\n'     'Move durable new evidence under docs/evidence/. If legacy snapshots were intentionally removed or moved, update the baseline in the same reviewed change.' >&2
  exit 1
fi

printf 'documentation structure: PASS (legacy snapshots=%s)\n' "$current"
