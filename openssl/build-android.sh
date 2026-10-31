#!/bin/sh
# OpenSSL shared libraries for Android arm64 (API 21+) -> dist/android-arm64/{libbmp_ssl.so,libbmp_crypto.so}.
#
# The sonames are renamed so they can never be confused with the system's private
# libssl.so/libcrypto.so, which apps may not load. libssl's DT_NEEDED is pointed at
# libbmp_crypto.so. MultiplayerAPI loads crypto first, by absolute path from the staged folder.
# Needs ANDROID_NDK_HOME and patchelf.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/.."
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK root}"
command -v patchelf >/dev/null || { echo "patchelf is required" >&2; exit 1; }
src="$("$here/fetch.sh" android-arm64)"
host_tag="$(ls "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" | head -1)"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"  # what OpenSSL's Configure reads
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_tag/bin:$PATH"
cd "$src"
./Configure android-arm64 shared no-tests no-docs no-apps no-engine no-module -D__ANDROID_API__=21 >/dev/null
make -j"$(getconf _NPROCESSORS_ONLN)" build_libs >/dev/null
out="$root/dist/android-arm64"
mkdir -p "$out"
cp libcrypto.so "$out/libbmp_crypto.so"
cp libssl.so "$out/libbmp_ssl.so"
cd "$out"
patchelf --set-soname libbmp_crypto.so libbmp_crypto.so
patchelf --set-soname libbmp_ssl.so libbmp_ssl.so
old_needed="$(llvm-readelf -d libbmp_ssl.so | sed -n 's/.*NEEDED.*\[\(libcrypto[^]]*\)\].*/\1/p')"
patchelf --replace-needed "$old_needed" libbmp_crypto.so libbmp_ssl.so
llvm-strip --strip-unneeded libbmp_crypto.so libbmp_ssl.so
cp "$src/LICENSE.txt" "$out/LICENSE-OpenSSL.txt"
