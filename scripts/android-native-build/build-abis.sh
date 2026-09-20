#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
: "${NATIVE_CRATE_DIR:=$root}"
: "${ANDROID_NDK_HOME:=${ANDROID_HOME:-}/ndk/27.2.12479018}"
: "${NATIVE_ABIS:=arm64-v8a,x86_64}"

"$root/scripts/android-native-build/preflight.sh"
[ -d "$ANDROID_NDK_HOME" ] || { echo "missing NDK: $ANDROID_NDK_HOME" >&2; exit 4; }
# The installed cargo-ndk release is a cargo subcommand.  Calling the wrapper
# binary directly is intentionally rejected by recent releases (and can
# otherwise make a packaging run look like an ABI/toolchain failure).
command -v cargo >/dev/null || { echo 'cargo is required for ABI builds' >&2; exit 4; }
cargo ndk --version >/dev/null 2>&1 || {
  echo 'cargo ndk is required for ABI builds' >&2
  exit 4
}
case ",$NATIVE_ABIS," in
  *,,*)
    echo 'NATIVE_ABIS must contain one or more comma-separated ABIs' >&2
    exit 4
    ;;
esac

old_ifs=$IFS
IFS=,
set -- $NATIVE_ABIS
IFS=$old_ifs
for abi do
  case "$abi" in
    arm64-v8a|x86_64) ;;
    *)
      echo "unsupported ABI: $abi (allowed: arm64-v8a,x86_64)" >&2
      exit 4
      ;;
  esac
  cargo ndk -t "$abi" build --release --manifest-path "$NATIVE_CRATE_DIR/Cargo.toml"
done
output="$root/android/app/build/generated/jniLibs"
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
rm -rf "$output"
old_ifs=$IFS
IFS=,
set -- $NATIVE_ABIS
IFS=$old_ifs
for abi do
  mkdir -p "$output/$abi"
  case "$abi" in
    arm64-v8a) target=aarch64-linux-android ;;
    x86_64) target=x86_64-linux-android ;;
  esac
  cp "$target_dir/$target/release/libany_cal_android_bridge.so" "$output/$abi/"
done
echo 'android-native-build ABI compilation: PASS'
