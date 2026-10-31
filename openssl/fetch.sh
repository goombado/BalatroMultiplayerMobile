#!/bin/sh
# Downloads the pinned OpenSSL tarball into openssl/.cache/ and verifies its SHA256.
# Prints the path of a fresh source tree extracted under openssl/build/<name>/.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/version.env"
name="${1:?usage: fetch.sh <build-name>}"
tarball="$here/.cache/openssl-$OPENSSL_VERSION.tar.gz"
mkdir -p "$here/.cache"
if [ ! -f "$tarball" ]; then
	curl -fsSL -o "$tarball.part" "https://github.com/openssl/openssl/releases/download/openssl-$OPENSSL_VERSION/openssl-$OPENSSL_VERSION.tar.gz"
	mv "$tarball.part" "$tarball"
fi
echo "$OPENSSL_SHA256  $tarball" | shasum -a 256 -c - >&2
rm -rf "$here/build/$name"
mkdir -p "$here/build/$name"
tar -xzf "$tarball" -C "$here/build/$name"
echo "$here/build/$name/openssl-$OPENSSL_VERSION"
