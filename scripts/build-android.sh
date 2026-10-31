#!/bin/sh
# Builds libbmp_steam.so for Android arm64 (API 21+) into dist/android-arm64/.
# Needs the Android NDK (ANDROID_NDK_HOME) and cargo-ndk.
set -eu
cd "$(dirname "$0")/.."
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK root}"
cargo ndk --target arm64-v8a --platform 21 build --release --locked -p bmp-steam-bridge
mkdir -p dist/android-arm64
cp target/aarch64-linux-android/release/libbmp_steam.so dist/android-arm64/
