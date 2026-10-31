#!/bin/sh
# Regenerates include/bmp_steam_bridge.h from the #[no_mangle] exports. Run it after changing the C API.
set -eu
cd "$(dirname "$0")/.."
mkdir -p include
cbindgen --config bridge/cbindgen.toml --crate bmp-steam-bridge --output include/bmp_steam_bridge.h bridge
