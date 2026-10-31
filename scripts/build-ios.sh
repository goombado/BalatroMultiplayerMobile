#!/bin/sh
# Builds libbmp_steam.dylib for iOS arm64 into dist/ios/.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked --target aarch64-apple-ios -p bmp-steam-bridge
mkdir -p dist/ios
cp target/aarch64-apple-ios/release/libbmp_steam.dylib dist/ios/
install_name_tool -id @rpath/libbmp_steam.dylib dist/ios/libbmp_steam.dylib
# Ad-hoc signature; LiveContainer / Sideloadly re-sign when injecting.
codesign -f -s - dist/ios/libbmp_steam.dylib
