#!/usr/bin/env bash
set -euo pipefail
prefix=${ANY_CAL_PREFIX:-"$HOME/.local"}
stage=${1:?usage: $0 STAGE_DIR}
[[ -n "$prefix" && "$prefix" != "/" ]] || { echo "refusing unsafe install prefix" >&2; exit 2; }
[[ -d "$stage" && -f "$stage/.any-cal-stage" ]] || { echo "invalid or unmarked stage directory" >&2; exit 2; }
[[ -x "$stage/bin/any-cal" ]] || { echo "invalid stage directory" >&2; exit 2; }
[[ -x "$stage/bin/any-cal-gui" ]] || { echo "invalid stage: missing any-cal-gui" >&2; exit 2; }
[[ -f "$stage/MANIFEST" && -s "$stage/MANIFEST" ]] || { echo "invalid stage: missing MANIFEST" >&2; exit 2; }
[[ -f "$stage/SHA256SUMS" && -s "$stage/SHA256SUMS" ]] || { echo "invalid stage: missing SHA256SUMS" >&2; exit 2; }
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$stage" && sha256sum -c SHA256SUMS >/dev/null)
elif command -v shasum >/dev/null 2>&1; then
  (cd "$stage" && shasum -a 256 -c SHA256SUMS >/dev/null)
else
  echo "no SHA-256 utility found (sha256sum or shasum)" >&2
  exit 1
fi

# Stage all managed files before activation. Existing files are moved to a
# same-filesystem backup and restored by the EXIT trap if activation fails.
# User configuration/checkpoints and unrelated files are never in this list.
prefix_parent=$(CDPATH= cd -- "$(dirname -- "$prefix")" && pwd)
prefix_name=$(basename -- "$prefix")
install_txn=$(mktemp -d "$prefix_parent/.${prefix_name}.install.XXXXXX")
install_backup="$install_txn/backup"
install_new="$install_txn/new"
mkdir -p "$install_backup" "$install_new/bin" "$install_new/share/doc/any-cal"
cleanup_install() {
  status=$?
  trap - EXIT
  if (( status != 0 )); then
    for target in \
      "$prefix/bin/any-cal" \
      "$prefix/bin/any-cal-gui" \
      "$prefix/share/doc/any-cal/README.md" \
      "$prefix/share/doc/any-cal/GUI.md"; do
      rm -f -- "$target"
    done
    if [[ -d "$install_backup" ]]; then
      while IFS= read -r -d '' backup; do
        relative=${backup#"$install_backup/"}
        target="$prefix/$relative"
        install -d "$(dirname -- "$target")"
        mv -- "$backup" "$target"
      done < <(find "$install_backup" -type f -print0)
    fi
  fi
  rm -rf -- "$install_txn"
  exit "$status"
}
trap cleanup_install EXIT

install -m 0755 "$stage/bin/any-cal" "$install_new/bin/any-cal"
install -m 0755 "$stage/bin/any-cal-gui" "$install_new/bin/any-cal-gui"
install -m 0644 "$stage/share/doc/any-cal/README.md" "$install_new/share/doc/any-cal/README.md"
install -m 0644 "$stage/share/doc/any-cal/GUI.md" "$install_new/share/doc/any-cal/GUI.md"
for relative in \
  bin/any-cal \
  bin/any-cal-gui \
  share/doc/any-cal/README.md \
  share/doc/any-cal/GUI.md; do
  target="$prefix/$relative"
  if [[ -e "$target" || -L "$target" ]]; then
    install -d "$install_backup/$(dirname -- "$relative")"
    mv -- "$target" "$install_backup/$relative"
  fi
  install -d "$(dirname -- "$target")"
  mv -- "$install_new/$relative" "$target"
done
trap - EXIT
rm -rf -- "$install_txn"
echo "installed any-cal to $prefix (existing configuration was not modified)"
