#!/usr/bin/env bash
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stage=""
version="$(awk -F= '/^version[[:space:]]*=/{gsub(/[[:space:]"]/,"",$2); print $2; exit}' "$root/Cargo.toml")"
skip_build=0
while (($#)); do
  case "$1" in
    --stage) stage=${2:?missing stage directory}; shift 2 ;;
    --version) version=${2:?missing version}; shift 2 ;;
    --skip-build) skip_build=1; shift ;;
    *) echo "usage: $0 --stage DIR [--version VERSION] [--skip-build]" >&2; exit 2 ;;
  esac
done
[[ -n "$stage" ]] || { echo "--stage is required" >&2; exit 2; }
stage=$(CDPATH= cd -- "$(dirname -- "$stage")" && pwd)/$(basename -- "$stage")
case "$stage" in
  /|"$root")
    echo "refusing unsafe source/workspace stage path: $stage" >&2
    exit 2
    ;;
  "$root"/*)
    case "$stage" in
      "$root"/tmp/*) ;;
      *)
        echo "refusing unsafe source/workspace stage path: $stage" >&2
        exit 2
        ;;
    esac
    ;;
esac
if [[ -e "$stage" ]]; then
  [[ -f "$stage/.any-cal-stage" ]] || { echo "refusing to remove unmarked existing directory: $stage" >&2; exit 2; }
fi
if (( ! skip_build )); then
  (cd "$root" && env -u LD_PRELOAD cargo build --release --offline -p any-cal-app -p any-cal-gui)
  printf '%s\n' "$version" > "$root/target/release/.any-cal-release-version"
fi
binary="$root/target/release/any-cal-app"
[[ -x "$binary" ]] || { echo "missing release binary: $binary" >&2; exit 1; }
gui_binary="$root/target/release/any-cal-gui"
[[ -x "$gui_binary" ]] || { echo "missing release GUI binary: $gui_binary" >&2; exit 1; }
if (( skip_build )); then
  marker="$root/target/release/.any-cal-release-version"
  [[ -f "$marker" ]] || { echo "--skip-build requires a release marker; run without it first" >&2; exit 1; }
  [[ "$(tr -d '[:space:]' < "$marker")" == "$version" ]] || { echo "stale release artifacts: marker/version mismatch" >&2; exit 1; }
fi
rm -rf "$stage"
install -d "$stage/bin" "$stage/share/doc/any-cal" "$stage/share/examples"
touch "$stage/.any-cal-stage"
install -m 0755 "$binary" "$stage/bin/any-cal"
install -m 0755 "$gui_binary" "$stage/bin/any-cal-gui"
install -m 0644 "$root/crates/app/README.md" "$stage/share/doc/any-cal/README.md"
install -m 0644 "$root/crates/ui/README.md" "$stage/share/doc/any-cal/GUI.md"
install -m 0644 "$root/packaging/any-cal.env.example" "$stage/share/examples/any-cal.env.example"
{
  echo "name=any-cal"
  echo "version=$version"
  echo "platform=$(uname -s)-$(uname -m)"
  find "$stage" -type f -print | sed "s#^$stage/##" | LC_ALL=C sort | while read -r path; do
    [[ "$path" == ".any-cal-stage" ]] && continue
    echo "artifact=$path"
  done
} > "$stage/MANIFEST"
if command -v sha256sum >/dev/null 2>&1; then
  checksum_cmd=sha256sum
elif command -v shasum >/dev/null 2>&1; then
  checksum_cmd='shasum -a 256'
else
  echo "no SHA-256 utility found (sha256sum or shasum)" >&2
  exit 1
fi
(cd "$stage" && $checksum_cmd $(find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort) > SHA256SUMS)
echo "staged any-cal $version in $stage"
