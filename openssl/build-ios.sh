#!/bin/sh
# OpenSSL shared libraries for iOS arm64 -> dist/ios/{libssl.3.dylib,libcrypto.3.dylib}.
#
# Install names are @loader_path/..., and libssl's dependency on libcrypto points to the same
# folder. Wherever the three dylibs are injected (LiveContainer tweak folder, app Frameworks/),
# MultiplayerAPI's existing macOS loader finds them through BET_OPENSSL_DYLIB_DIR, which the
# bridge's load-time constructor sets to that folder.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/.."
src="$("$here/fetch.sh" ios)"
cd "$src"
./Configure ios64-xcrun shared no-tests no-docs no-apps no-engine no-module -mios-version-min=14.0 >/dev/null
make -j"$(sysctl -n hw.ncpu)" build_libs >/dev/null
out="$root/dist/ios"
mkdir -p "$out"
cp libcrypto.3.dylib libssl.3.dylib "$out/"
cd "$out"
old_crypto="$(otool -D libcrypto.3.dylib | tail -1)"
install_name_tool -id @loader_path/libcrypto.3.dylib libcrypto.3.dylib
install_name_tool -id @loader_path/libssl.3.dylib libssl.3.dylib
install_name_tool -change "$old_crypto" @loader_path/libcrypto.3.dylib libssl.3.dylib
codesign -f -s - libcrypto.3.dylib libssl.3.dylib
cp "$src/LICENSE.txt" "$out/LICENSE-OpenSSL.txt"
