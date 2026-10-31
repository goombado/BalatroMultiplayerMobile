#!/bin/sh
# Builds the native libraries into dist/: the Steam bridge and OpenSSL.
#   scripts/build.sh --ios | --android | --all
# iOS needs Xcode; Android needs ANDROID_NDK_HOME, cargo-ndk and patchelf.
set -eu
cd "$(dirname "$0")/.."
case "${1:-}" in
	--ios) targets=ios ;;
	--android) targets=android ;;
	--all) targets="ios android" ;;
	*) echo "usage: scripts/build.sh --ios | --android | --all" >&2; exit 2 ;;
esac
for t in $targets; do
	echo "== $t"
	"./scripts/build-$t.sh"
	"./openssl/build-$t.sh"
done
