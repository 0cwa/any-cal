#!/bin/sh
set -eu

: "${APK_PATH:?set APK_PATH to the release APK}"
: "${ANDROID_ABIS:=arm64-v8a,x86_64}"
: "${EXPECTED_PACKAGE:=org.anycal.android}"

[ -f "$APK_PATH" ] || { echo "APK not found: $APK_PATH" >&2; exit 2; }
command -v unzip >/dev/null || { echo 'unzip is required' >&2; exit 2; }
command -v sha256sum >/dev/null || { echo 'sha256sum is required' >&2; exit 2; }

aapt_bin=${AAPT_BIN:-}
if [ -z "$aapt_bin" ]; then
  if command -v aapt >/dev/null 2>&1; then
    aapt_bin=$(command -v aapt)
  elif [ -n "${ANDROID_HOME:-}" ] && [ -x "$ANDROID_HOME/build-tools/34.0.0/aapt" ]; then
    aapt_bin="$ANDROID_HOME/build-tools/34.0.0/aapt"
  else
    echo 'aapt is required for APK metadata assertions' >&2
    exit 2
  fi
fi
[ -x "$aapt_bin" ] || { echo "aapt not executable: $aapt_bin" >&2; exit 2; }

case ",$ANDROID_ABIS," in
  *,,*)
    echo 'ANDROID_ABIS must contain one or more comma-separated ABIs' >&2
    exit 4
    ;;
esac

entries=$(unzip -Z1 "$APK_PATH")
old_ifs=$IFS
IFS=,
set -- $ANDROID_ABIS
IFS=$old_ifs
for abi do
  case "$abi" in
    arm64-v8a|x86_64) ;;
    *)
      echo "unsupported ABI: $abi (allowed: arm64-v8a,x86_64)" >&2
      exit 4
      ;;
  esac
  entry="lib/$abi/libany_cal_android_bridge.so"
  if printf '%s\n' "$entries" | grep -Fqx "$entry"; then
    echo "apk-entry=$entry:present"
  else
    echo "apk-entry=$entry:missing" >&2
    exit 3
  fi
done

badging=$($aapt_bin dump badging "$APK_PATH")
package_line=$(printf '%s\n' "$badging" | sed -n 's/^package: //p')
case "$package_line" in
  *"name='$EXPECTED_PACKAGE'"*) ;;
  *)
    echo "APK package metadata does not contain name='$EXPECTED_PACKAGE'" >&2
    exit 3
    ;;
esac
printf '%s\n' "$badging" | grep -Fqx "sdkVersion:'26'" &&
  printf '%s\n' "$badging" | grep -Fqx "targetSdkVersion:'35'" || {
  echo 'APK SDK metadata does not declare minSdk 26 and targetSdk 35' >&2
  exit 3
}
printf '%s\n' "$badging" | grep -Eq "^native-code:.*'arm64-v8a'" &&
  printf '%s\n' "$badging" | grep -Eq "^native-code:.*'x86_64'" || {
  echo 'APK native-code metadata does not declare both arm64-v8a and x86_64' >&2
  exit 3
}
echo "apk-metadata=package:$EXPECTED_PACKAGE minSdk:26 targetSdk:35"
echo "APK_SHA256=$(sha256sum "$APK_PATH" | awk '{print $1}')"
echo 'android-release-apk assertion: PASS'
