#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-}}"
: "${GRADLE_BIN:=gradle}"
: "${NATIVE_CRATE_DIR:=$root/crates/android-bridge}"

"$root/scripts/android-build/preflight.sh"
# A fresh checkout has no generated JNI libraries. Build the bridge for every
# ABI that the Android module declares before Gradle packages the APK.
ANDROID_HOME="$ANDROID_HOME" \
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.2.12479018}" \
GRADLE_BIN="$GRADLE_BIN" \
NATIVE_CRATE_DIR="$NATIVE_CRATE_DIR" \
  "$root/scripts/android-native-build/build-abis.sh"
cd "$root/android"
"$GRADLE_BIN" --no-daemon :app:assembleDebug :app:assembleRelease :app:testDebugUnitTest
python3 "$root/android/app/src/test/tasks/verify_tasks_org_contract.py"

apk="$root/android/app/build/outputs/apk/release/app-release-unsigned.apk"
[ -f "$apk" ] || { echo "APK not found: $apk" >&2; exit 3; }
APK_PATH="$apk" ANDROID_ABIS=arm64-v8a,x86_64 \
  "$root/scripts/android-native-build/assert-apk.sh"
echo 'android-build assemble/unit/verifier: PASS'
