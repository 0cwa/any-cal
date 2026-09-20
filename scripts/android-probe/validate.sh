#!/bin/sh
set -eu
base=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python3 -m py_compile "$base/probe.py"
python3 "$base/probe.py" --help >/dev/null
python3 "$base/test_probe.py"
echo "android-probe offline validation: PASS"
