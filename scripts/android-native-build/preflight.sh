#!/bin/sh
set -eu

: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-}}"
: "${GRADLE_BIN:=gradle}"

command -v java >/dev/null || { echo 'missing java' >&2; exit 2; }
command -v javac >/dev/null || { echo 'missing javac (JDK required)' >&2; exit 2; }
java -version 2>&1 | sed -n '1p'
javac -version
if command -v cargo >/dev/null; then cargo --version; else echo 'cargo=missing'; fi
if command -v cargo >/dev/null; then
  if cargo ndk --version >/dev/null 2>&1; then
    cargo ndk --version
  else
    echo 'cargo-ndk=cargo-subcommand-missing'
  fi
else
  echo 'cargo=missing'
  echo 'cargo-ndk=cargo-subcommand-unavailable'
fi
if [ -n "$ANDROID_HOME" ] && [ -d "$ANDROID_HOME" ]; then
  echo "ANDROID_HOME=$ANDROID_HOME"
  find "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -type d -printf 'ndk=%f\n' 2>/dev/null || echo 'ndk=missing'
else
  echo 'ANDROID_HOME=missing'
  echo 'ndk=missing'
fi
if command -v "$GRADLE_BIN" >/dev/null; then "$GRADLE_BIN" --version | sed -n '1,4p'; else echo 'gradle=missing'; fi

native_crates=0
if find . -name Cargo.toml -type f -print0 2>/dev/null | xargs -0 grep -l 'crate-type.*cdylib' >/dev/null 2>&1; then
  native_crates=1
fi
echo "cdylib_crate=$native_crates"
if [ "$native_crates" -eq 0 ]; then
  echo 'native-artifact-source=missing'
  echo 'android-native-build preflight: environment inspected; native source is not ready'
  exit 3
fi
echo 'android-native-build preflight: PASS'
