#!/usr/bin/env bash
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp_root=${ANY_CAL_TMP_ROOT:-"$root/tmp"}
mkdir -p -- "$tmp_root"
tmp=$(mktemp -d "$tmp_root/packaging.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
# Build once so a clean checkout creates and verifies the release marker.
env -u LD_PRELOAD "$root/packaging/release.sh" --stage "$tmp/stage"
test -x "$tmp/stage/bin/any-cal"
test -x "$tmp/stage/bin/any-cal-gui"
test -s "$tmp/stage/MANIFEST"
test -s "$tmp/stage/SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$tmp/stage" && sha256sum -c SHA256SUMS)
else
  (cd "$tmp/stage" && shasum -a 256 -c SHA256SUMS)
fi
# Reject token assignments, while allowing documentation that discusses the
# environment variable without treating the literal example as a credential.
! rg -n '(^|[[:space:]])(ANY_CAL_ANYTYPE_TOKEN|token)[[:space:]]*=[^#[:space:]]+' "$tmp/stage" || { echo 'secret-like token leaked' >&2; exit 1; }
mkdir -p "$tmp/prefix"
printf 'space_id=keep-me\n' > "$tmp/prefix/any-cal.conf"
mkdir -p "$tmp/prefix/state"
printf 'checkpoint=17\n' > "$tmp/prefix/state/checkpoint"
printf 'unrelated\n' > "$tmp/prefix/unrelated.txt"
ANY_CAL_PREFIX="$tmp/prefix" "$root/packaging/install.sh" "$tmp/stage"
test -x "$tmp/prefix/bin/any-cal"
test -f "$tmp/prefix/any-cal.conf"
test "$(cat "$tmp/prefix/state/checkpoint")" = 'checkpoint=17'
test "$(cat "$tmp/prefix/unrelated.txt")" = 'unrelated'

# An altered staged artifact must be rejected before activation and must not
# disturb the currently installed version or user-owned state.
cp "$tmp/prefix/bin/any-cal" "$tmp/installed-any-cal"
printf 'tampered\n' >> "$tmp/stage/bin/any-cal"
if ANY_CAL_PREFIX="$tmp/prefix" "$root/packaging/install.sh" "$tmp/stage" >/dev/null 2>&1; then
  echo 'checksum-tampered stage was accepted' >&2
  exit 1
fi
cmp "$tmp/installed-any-cal" "$tmp/prefix/bin/any-cal"
test "$(cat "$tmp/prefix/state/checkpoint")" = 'checkpoint=17'

ANY_CAL_PREFIX="$tmp/prefix" "$root/packaging/uninstall.sh"
test ! -e "$tmp/prefix/bin/any-cal"
test ! -e "$tmp/prefix/bin/any-cal-gui"
test -f "$tmp/prefix/any-cal.conf"
test -f "$tmp/prefix/state/checkpoint"
test -f "$tmp/prefix/unrelated.txt"
mkdir -p "$tmp/unmarked"
printf 'keep\n' > "$tmp/unmarked/sentinel"
if env -u LD_PRELOAD "$root/packaging/release.sh" --stage "$tmp/unmarked" --skip-build; then
  echo 'unsafe unmarked stage was accepted' >&2
  exit 1
fi
test -f "$tmp/unmarked/sentinel"
echo 'packaging smoke test passed'
