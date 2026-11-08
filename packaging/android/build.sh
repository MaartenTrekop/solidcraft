#!/usr/bin/env bash
# Build the SolidCraft APK: the Rust library for arm64 phones (cargo-ndk), then the APK around it
# (Gradle). Needs the Android SDK and NDK (ANDROID_HOME, ANDROID_NDK_HOME), Java 17, Gradle 8.9 or
# newer, the aarch64-linux-android Rust target and cargo-ndk 3:
#
#   rustup target add aarch64-linux-android
#   cargo install cargo-ndk --version "^3" --locked
#
# Result: packaging/android/app/build/outputs/apk/release/app-release.apk
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

# Line tables make the library several times larger and the phone gains nothing from them.
export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-false}"
export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-true}"

cd "$root"
cargo ndk --target arm64-v8a --platform 30 --output-dir "$here/app/src/main/jniLibs" \
    build --release -p solidcraft-android

gradle -p "$here" assembleRelease
echo "APK: $here/app/build/outputs/apk/release/app-release.apk"
