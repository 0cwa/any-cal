#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
run_android=0
fetch_dependencies=1

usage() {
  cat <<'EOF'
usage: bash scripts/validate.sh [--offline] [--android]

Runs the repository and standalone Anytype probe pre-PR validation profile.

  --offline  Skip both Cargo fetches; require both workspaces' dependencies cached.
  --android  Also run the full Android native/APK/unit-test build harness.
EOF
}

while (($#)); do
  case "$1" in
    --offline)
      fetch_dependencies=0
      ;;
    --android)
      run_android=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  shift
done

cd "$root"
mkdir -p "$root/tmp"
export ANY_CAL_TMP_ROOT="${ANY_CAL_TMP_ROOT:-$root/tmp}"

printf '%s\n' '==> documentation structure'
bash scripts/docs/check-structure.sh

printf '%s\n' '==> offline Android probe contract'
scripts/android-probe/validate.sh

printf '%s\n' '==> Rust formatting'
env -u LD_PRELOAD cargo fmt --all -- --check

printf '%s\n' '==> standalone Anytype probe formatting'
env -u LD_PRELOAD cargo fmt --manifest-path tools/anytype-probe/Cargo.toml --all -- --check

if (( fetch_dependencies )); then
  printf '%s\n' '==> locked dependency fetch'
  env -u LD_PRELOAD cargo fetch --locked
  env -u LD_PRELOAD cargo fetch --manifest-path tools/anytype-probe/Cargo.toml --locked
else
  printf '%s\n' '==> locked dependency fetch skipped (--offline)'
fi

printf '%s\n' '==> workspace tests'
env -u LD_PRELOAD cargo test --workspace --offline --locked

printf '%s\n' '==> standalone Anytype probe tests'
env -u LD_PRELOAD cargo test --manifest-path tools/anytype-probe/Cargo.toml --offline --locked

printf '%s\n' '==> Clippy'
env -u LD_PRELOAD cargo clippy --workspace --all-targets --offline --locked -- -D warnings

printf '%s\n' '==> standalone Anytype probe Clippy'
env -u LD_PRELOAD cargo clippy --manifest-path tools/anytype-probe/Cargo.toml --all-targets --offline --locked -- -D warnings

printf '%s\n' '==> packaging smoke test'
env -u LD_PRELOAD scripts/test-packaging.sh

if (( run_android )); then
  printf '%s\n' '==> full Android build'
  env -u LD_PRELOAD scripts/android-build/build-and-test.sh
fi

printf '%s\n' 'validation: PASS'
