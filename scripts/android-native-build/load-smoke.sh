#!/bin/sh
set -eu

: "${APK_PATH:?set APK_PATH to a debug APK containing native libraries}"
: "${ANDROID_ABIS:=arm64-v8a,x86_64}"

[ -f "$APK_PATH" ] || { echo "APK not found: $APK_PATH" >&2; exit 2; }
command -v unzip >/dev/null || { echo 'unzip is required' >&2; exit 2; }
missing=0
old_ifs=$IFS
IFS=,
set -- $ANDROID_ABIS
IFS=$old_ifs
for abi do
  if unzip -Z1 "$APK_PATH" | grep -Eq "^lib/$abi/[^/]+\.so$"; then
    echo "native-library=$abi:present"
  else
    echo "native-library=$abi:missing"
    missing=1
  fi
done
[ "$missing" -eq 0 ] || exit 3
echo 'android-native-load smoke preflight: PASS'
