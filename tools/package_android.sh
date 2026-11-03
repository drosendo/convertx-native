#!/usr/bin/env bash
# Builds dist/ConvertX.apk. Needs the Android SDK + NDK and a JDK (aapt2 and
# apksigner live in build-tools). `cargo install cargo-ndk` first.
#
#   export ANDROID_HOME="$HOME/Library/Android/sdk"
#   export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
#   ./tools/package_android.sh
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="${HOME}/.cargo/bin:${PATH}"

# shellcheck source=/dev/null
. tools/android_env.sh

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "$SDK" ] || [ ! -d "$SDK" ]; then
  echo "error: set ANDROID_HOME to your Android SDK" >&2
  exit 1
fi

BUILD_TOOLS="$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)"
JAR="$(ls -d "$SDK"/platforms/android-*/android.jar 2>/dev/null | sort -V | head -1)"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

if [ -z "$BUILD_TOOLS" ] || [ -z "$JAR" ]; then
  echo "error: build-tools or platforms missing — run: sdkmanager 'build-tools;35.0.0' 'platforms;android-34'" >&2
  exit 1
fi

ABIS="${ABIS:-arm64-v8a armeabi-v7a}"

echo "Building the shared library for: $ABIS"
for abi in $ABIS; do
  case "$abi" in
    arm64-v8a) target=aarch64-linux-android ;;
    armeabi-v7a) target=armv7-linux-androideabi ;;
    x86_64) target=x86_64-linux-android ;;
    x86) target=i686-linux-android ;;
    *) echo "error: unknown ABI $abi" >&2; exit 1 ;;
  esac
  rustup target add "$target" >/dev/null 2>&1 || true
done
rm -rf android/lib
args=()
for abi in $ABIS; do args+=(-t "$abi"); done
cargo ndk "${args[@]}" -o android/lib build --release --lib

echo "Compiling the manifest…"
rm -rf build
mkdir -p build dist
"$BUILD_TOOLS/aapt2" link \
  -o build/base.apk \
  -I "$JAR" \
  --manifest android/AndroidManifest.xml \
  --min-sdk-version 24 \
  --target-sdk-version 34 \
  --version-code 1 \
  --version-name "$VERSION"

echo "Adding native libraries…"
if ! ls android/lib/*/lib*.so >/dev/null 2>&1; then
  echo "error: cargo-ndk produced no shared library" >&2
  find android/lib -type f 2>/dev/null || true
  exit 1
fi
# cargo-ndk writes android/lib/<abi>/libconvertx.so; the APK wants lib/<abi>/…
(cd android && zip -q -r "../build/base.apk" lib)

echo "Aligning…"
"$BUILD_TOOLS/zipalign" -f -p 4 build/base.apk build/aligned.apk

echo "Signing…"
KEYSTORE="build/debug.keystore"
if [ ! -f "$KEYSTORE" ]; then
  keytool -genkeypair -v \
    -keystore "$KEYSTORE" \
    -alias androiddebugkey \
    -storepass android -keypass android \
    -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=ConvertX Debug,O=Rosendo Labs,C=PT" >/dev/null 2>&1
fi
"$BUILD_TOOLS/apksigner" sign \
  --ks "$KEYSTORE" \
  --ks-pass pass:android \
  --key-pass pass:android \
  --out "dist/ConvertX-$VERSION.apk" \
  "build/aligned.apk"

echo "Done: dist/ConvertX-$VERSION.apk"
echo "Install with: adb install -r dist/ConvertX-$VERSION.apk"
