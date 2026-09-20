#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

path_class() {
  local path="$1"
  case "$path" in
    .local|.local/*) printf '%s\n' 'private-state' ;;
    tmp|tmp/*) printf '%s\n' 'workspace-scratch' ;;
    target|target/*) printf '%s\n' 'rust-build-output' ;;
    .gradle|.gradle/*) printf '%s\n' 'gradle-state' ;;
    android/.gradle|android/.gradle/*) printf '%s\n' 'android-gradle-state' ;;
    android/*/build|android/*/build/*) printf '%s\n' 'android-build-output' ;;
    scripts/__pycache__|scripts/__pycache__/*) printf '%s\n' 'python-cache' ;;
    .env|*/.env|.env.*|*/.env.*|credentials.env|*/credentials.env|*.pem|*/.pem|*.p12|*/.p12)
      printf '%s\n' 'credential-like-file'
      ;;
    *) return 1 ;;
  esac
}

check_paths() {
  local scope="$1"
  local bad=0
  local path class
  while IFS= read -r -d '' path; do
    class="$(path_class "$path" || true)"
    if [[ -n "$class" ]]; then
      printf 'publication path guard: scope=%s class=%s\n' "$scope" "$class" >&2
      bad=1
    fi
  done
  return "$bad"
}

bad=0
if ! check_paths index < <(git ls-files -z --cached); then
  bad=1
fi

# Keep the history check independent of gitleaks' path allowlist. A generated
# path that was committed and later removed is still part of the publication
# history and must not be silently exempted by gitleaks.toml.
while IFS=' ' read -r _object path; do
  [[ -n "$path" ]] || continue
  class="$(path_class "$path" || true)"
  if [[ -n "$class" ]]; then
    printf 'publication path guard: scope=history class=%s\n' "$class" >&2
    bad=1
  fi
done < <(git rev-list --objects --all)

if [[ "$bad" -ne 0 ]]; then
  printf '%s\n' 'publication path guard: blocked tracked or historical private/generated paths' >&2
  exit 1
fi

printf '%s\n' 'publication path guard: clean'
