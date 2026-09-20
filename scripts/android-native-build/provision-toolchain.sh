#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
: "${ANY_CAL_TMP_ROOT:=$root/tmp}"
mkdir -p -- "$ANY_CAL_TMP_ROOT"
: "${ANDROID_SDK_ROOT:=/opt/android-sdk}"
: "${ANDROID_NDK_VERSION:=27.2.12479018}"
: "${GRADLE_VERSION:=8.10.2}"
: "${RUST_TOOLCHAIN:=1.98.1}"
: "${CARGO_NDK_VERSION:=4.1.2}"

export ANDROID_HOME="$ANDROID_SDK_ROOT"
export PATH="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$ANDROID_SDK_ROOT/platform-tools:$HOME/.cargo/bin:$PATH"

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  ca-certificates curl unzip openjdk-17-jdk libxkbfile1 build-essential \
  pkg-config >"$ANY_CAL_TMP_ROOT/android-native-apt.log" 2>&1

mkdir -p "$ANDROID_SDK_ROOT/cmdline-tools" /opt/gradle
if [ ! -x "$ANDROID_SDK_ROOT/cmdline-tools/latest/bin/sdkmanager" ]; then
  curl -fL --retry 2 --silent --show-error \
    https://dl.google.com/android/repository/commandlinetools-linux-15859902_latest.zip \
    -o "$ANY_CAL_TMP_ROOT/android-cmdline-tools.zip"
  unzip -q "$ANY_CAL_TMP_ROOT/android-cmdline-tools.zip" -d "$ANDROID_SDK_ROOT/cmdline-tools"
  mv "$ANDROID_SDK_ROOT/cmdline-tools/cmdline-tools" "$ANDROID_SDK_ROOT/cmdline-tools/latest"
  rm -f "$ANY_CAL_TMP_ROOT/android-cmdline-tools.zip"
fi
yes | sdkmanager --licenses >/dev/null
sdkmanager --install \
  "platform-tools" "platforms;android-35" "build-tools;34.0.0" \
  "ndk;${ANDROID_NDK_VERSION}"

if [ ! -x "/opt/gradle/gradle-${GRADLE_VERSION}/bin/gradle" ]; then
  curl -fL --retry 2 --silent --show-error \
    "https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip" \
    -o "$ANY_CAL_TMP_ROOT/gradle.zip"
  unzip -q "$ANY_CAL_TMP_ROOT/gradle.zip" -d /opt/gradle
  rm -f "$ANY_CAL_TMP_ROOT/gradle.zip"
fi

if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain "$RUST_TOOLCHAIN" --profile minimal
fi
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal
rustup default "$RUST_TOOLCHAIN"
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk --version "$CARGO_NDK_VERSION" --locked

echo "ANDROID_SDK_ROOT=$ANDROID_SDK_ROOT"
echo "ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/$ANDROID_NDK_VERSION"
java -version 2>&1 | sed -n '1p'
javac -version
"/opt/gradle/gradle-${GRADLE_VERSION}/bin/gradle" --version | sed -n '1,4p'
rustc --version
cargo --version
cargo ndk --version
echo "NDK_VERSION=$ANDROID_NDK_VERSION"
echo "CARGO_NDK_VERSION=$CARGO_NDK_VERSION"
echo 'android-native-build toolchain provisioning: PASS'
