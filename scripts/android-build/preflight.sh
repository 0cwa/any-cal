#!/bin/sh
set -eu

: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-}}"
: "${GRADLE_BIN:=gradle}"

command -v java >/dev/null || { echo 'missing java' >&2; exit 2; }
command -v javac >/dev/null || { echo 'missing javac (install JDK, not only JRE)' >&2; exit 2; }
command -v "$GRADLE_BIN" >/dev/null || { echo "missing Gradle: $GRADLE_BIN" >&2; exit 2; }
[ -n "$ANDROID_HOME" ] || { echo 'set ANDROID_HOME or ANDROID_SDK_ROOT' >&2; exit 2; }
[ -d "$ANDROID_HOME/platforms/android-35" ] || { echo "missing SDK platform: $ANDROID_HOME/platforms/android-35" >&2; exit 2; }
[ -d "$ANDROID_HOME/build-tools/34.0.0" ] || { echo "missing Build-Tools: $ANDROID_HOME/build-tools/34.0.0" >&2; exit 2; }

java -version 2>&1 | sed -n '1p'
javac -version
"$GRADLE_BIN" --version | sed -n '1,4p'
echo "ANDROID_HOME=$ANDROID_HOME"
echo 'android-build preflight: PASS'
