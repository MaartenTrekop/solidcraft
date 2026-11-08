#!/usr/bin/env bash
# Build the SolidCraft APK: the Rust library for arm64 phones (cargo-ndk), then the APK around it
# with the Android SDK's own tools (aapt2, zipalign, apksigner). No Gradle, no Java code: the app is
# a NativeActivity.
#
# Needs: the Android SDK with build-tools 35 or newer and a platform (ANDROID_HOME), the NDK
# (ANDROID_NDK_HOME), Java for apksigner, zip, the aarch64-linux-android Rust target and cargo-ndk 3:
#
#   rustup target add aarch64-linux-android
#   cargo install cargo-ndk --version "^3" --locked
#
# SOLIDCRAFT_VERSION_CODE and SOLIDCRAFT_VERSION_NAME set the APK's version (1 and "dev").
# Result: packaging/android/build/SolidCraft.apk
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/build"
code="${SOLIDCRAFT_VERSION_CODE:-1}"
name="${SOLIDCRAFT_VERSION_NAME:-dev}"

sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "$sdk" ] || [ ! -d "$sdk/build-tools" ]; then
    echo "Set ANDROID_HOME to the Android SDK (with build-tools and a platform)." >&2
    exit 1
fi
tools="$(ls -d "$sdk"/build-tools/* | sort -V | tail -n 1)"
platform="$(ls -d "$sdk"/platforms/android-* | sort -V | tail -n 1)/android.jar"
echo "build-tools: $tools"
echo "platform: $platform"

rm -rf "$out"
mkdir -p "$out/stage/lib"

# The library. Line tables make it several times larger and the phone gains nothing from them.
export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-false}"
export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-true}"
(cd "$root" && cargo ndk --target arm64-v8a --platform 30 --output-dir "$out/stage/lib" \
    build --release -p solidcraft-android)

# Manifest and resources (the launcher icon).
"$tools/aapt2" compile --dir "$here/res" -o "$out/res.zip"
"$tools/aapt2" link -o "$out/base.apk" -I "$platform" --manifest "$here/AndroidManifest.xml" \
    --min-sdk-version 30 --target-sdk-version 34 --version-code "$code" --version-name "$name" \
    "$out/res.zip"

# The library, stored uncompressed (the manifest's extractNativeLibs="false"), then every
# uncompressed library aligned to 16 KB pages, then the signature.
(cd "$out/stage" && zip -q -0 -r "$out/base.apk" lib)
"$tools/zipalign" -f -P 16 4 "$out/base.apk" "$out/aligned.apk"
"$tools/apksigner" sign --ks "$here/sideload.keystore" --ks-type PKCS12 --ks-pass pass:android \
    --ks-key-alias androiddebugkey --key-pass pass:android --out "$out/SolidCraft.apk" "$out/aligned.apk"
"$tools/apksigner" verify "$out/SolidCraft.apk"
echo "APK: $out/SolidCraft.apk"
