# Native libraries (Android)

Release builds put the Android libraries here; the repository holds none.

| File | What |
|---|---|
| `android-arm64/libbmp_steam.so` | The Steam bridge (`bridge/` in this repository) |
| `android-arm64/libbmp_ssl.so`, `libbmp_crypto.so` | OpenSSL 3.6.x, renamed so they never clash with Android's private copies |
| `android-arm64/SHA256SUMS` | Hashes of the three |
| `android-arm64/LICENSE-OpenSSL.txt` | OpenSSL's Apache-2.0 licence, which has to ship with its binaries |

`scripts/package.sh` fills `android-arm64/` from `dist/` when it builds the mod zip. To
check a copy: `cd native/android-arm64 && shasum -a 256 -c SHA256SUMS`.

On Android, `src/native_stage.lua` copies them into app-private storage at startup,
because the dynamic linker can't load from a zip-mounted mod or from noexec external
storage. iOS gets the bridge and OpenSSL by dylib injection instead.
