#!/usr/bin/env bash
set -euo pipefail
prefix=${ANY_CAL_PREFIX:-"$HOME/.local"}
[[ -n "$prefix" && "$prefix" != "/" ]] || { echo "refusing unsafe uninstall prefix" >&2; exit 2; }
rm -f "$prefix/bin/any-cal" "$prefix/bin/any-cal-gui" \
  "$prefix/share/doc/any-cal/README.md" "$prefix/share/doc/any-cal/GUI.md"
rmdir "$prefix/share/doc/any-cal" 2>/dev/null || true
echo "removed any-cal binaries and packaged documentation from $prefix; Anytype data and configuration were untouched"
