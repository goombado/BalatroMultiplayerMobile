#!/bin/sh
# Assembles the release files from mod/ and the libraries in dist/ (build them
# first with scripts/build.sh --all):
#   dist/release/MultiplayerMobile-<version>.zip   the mod, with native/android-arm64/ filled in
#   dist/release/BMP-Mobile-iOS-<version>.zip      the three iOS dylibs, for a LiveContainer tweak folder
#   dist/release/SHA256SUMS
set -eu
cd "$(dirname "$0")/.."
version="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' mod/MultiplayerMobile.json)"
out=dist/release
stage="$out/stage"
rm -rf "$out"
mkdir -p "$stage"

sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

# The mod, without tests or editor files.
mkdir -p "$stage/MultiplayerMobile"
tar -C mod --exclude tests --exclude .gitignore --exclude stylua.toml -cf - . | tar -C "$stage/MultiplayerMobile" -xf -
native="$stage/MultiplayerMobile/native/android-arm64"
mkdir -p "$native"
cp dist/android-arm64/libbmp_steam.so dist/android-arm64/libbmp_ssl.so dist/android-arm64/libbmp_crypto.so dist/android-arm64/LICENSE-OpenSSL.txt "$native/"
(cd "$native" && sum libbmp_crypto.so libbmp_ssl.so libbmp_steam.so > SHA256SUMS)
cp LICENSE NOTICE "$stage/MultiplayerMobile/"
(cd "$stage" && zip -qr "../MultiplayerMobile-$version.zip" MultiplayerMobile)

# iOS tweak folder.
mkdir -p "$stage/BMP-Mobile"
cp dist/ios/libbmp_steam.dylib dist/ios/libssl.3.dylib dist/ios/libcrypto.3.dylib dist/ios/LICENSE-OpenSSL.txt "$stage/BMP-Mobile/"
(cd "$stage/BMP-Mobile" && sum libbmp_steam.dylib libssl.3.dylib libcrypto.3.dylib > SHA256SUMS)
(cd "$stage" && zip -qr "../BMP-Mobile-iOS-$version.zip" BMP-Mobile)

rm -rf "$stage"
(cd "$out" && sum *.zip > SHA256SUMS)
ls -l "$out"
