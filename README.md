# Multiplayer Mobile

Lets Balatro builds made with the Lovely mobile maker (iOS and Android) log in to Balatro Multiplayer's
**unchanged** official server with your own Steam account. Two parts:

- **The mod** (`mod/`, SMODS id `MultiplayerMobile`): installed next to MultiplayerAPI. It registers the
  bridge as MultiplayerAPI's Steam provider and adds the login screen. It does nothing on desktop.
- **The Steam bridge** (`bridge/`, Rust): a native library that logs in to Steam and mints tickets.

On desktop, MultiplayerAPI gets a Steam auth session ticket from the Steam client and the server checks it
with Valve (`ISteamUserAuth/AuthenticateUserTicket`). Phones have no Steam client. This library does the
client's part itself:

1. It logs into Steam over the Steam client protocol (CM WebSocket).
2. It mints a genuine Balatro (app 2379780) auth session ticket and registers it with Steam.
3. It hands the ticket out through the same Steamworks C functions MultiplayerAPI already looks up.

The server is not changed and can't tell the difference: the ticket is real, and it is checked the same way.

Ranked is out of scope: it needs the desktop anti-cheat launcher.

## Security model

- **Credentials:** your Steam password, refresh token and access tokens only ever go to Valve
  (`*.steamserver.net` CM sockets, `api.steampowered.com`). Balatro Multiplayer never sees them.
  QR login is recommended: no password is typed on the phone at all.
- **Stored login:** a SteamClient refresh token, about 200 days.
  - iOS: the Keychain. It falls back to a 0600 file in the save dir if the Keychain is unavailable.
  - Android: a 0600 file in the app-private save dir, protected by the app sandbox only.
- **Logout:** logging out deletes the token.
- **Redaction:** every secret is registered with a redactor, and everything written to the log
  (`bmp_steam_bridge.log` in the save dir) or returned as an error passes through it. JWT-shaped
  strings are scrubbed too.
- **What Steam sees:** a Steam client session for your account, named
  "Balatro Multiplayer (iOS|Android)" in Steam's authorized devices. By default it does not show you
  as in-game. Set `BMP_BRIDGE_GAMES_PLAYED=1` to report Balatro as played; tickets validate without it.

## ABI

See `include/bmp_steam_bridge.h` (generated, ABI version 1).

- **Steamworks-compatible exports:** `SteamAPI_SteamUser_v020`…`v023`,
  `SteamAPI_ISteamUser_GetAuthSessionTicket` and `SteamAPI_ISteamUser_CancelAuthTicket`.
- **Login API:** `bmp_bridge_*`, polled; there are no callbacks.
- **Speed:** every call returns in microseconds. A pool of two pre-activated tickets lets
  `GetAuthSessionTicket` answer synchronously, as Steamworks does.

On iOS a load-time constructor exports the library's own folder as `BET_OPENSSL_DYLIB_DIR` (unless
it is already set), plus `BMP_STEAM_BRIDGE=1`. That lets MultiplayerAPI's existing macOS OpenSSL
loader find `libssl.3.dylib`/`libcrypto.3.dylib` injected next to it.

## Layout

| Path | What |
|---|---|
| `mod/` | The SMODS mod (Lua): `main.lua`, `src/`, `localization/`, `lib/qrencode/` (vendored), `tests/` |
| `mod/native/` | Empty in the repository; the release build puts the Android libraries here |
| `bridge/` | The library (`cdylib` + `rlib`, crate `bmp-steam-bridge`, lib name `bmp_steam`) |
| `bridge-cli/` | Desktop harness that drives the same C ABI |
| `vendor/steam-vent/` | steam-vent 0.5.0 with one patch (`vendor/PATCHES.md`) |
| `openssl/` | Pinned OpenSSL 3.6.5 builds for iOS/Android and a symbol check against MultiplayerAPI's cdefs |
| `scripts/` | `build.sh --ios / --android / --all`, `package.sh` (release zips), header generation |
| `docs/` | Install guide, device test checklist |

## Building

The repository holds no binaries: build them locally and attach the `scripts/package.sh` output to a release.
Locally you need Rust (see `rust-toolchain.toml`), Xcode for iOS, and for Android the NDK
(`ANDROID_NDK_HOME`), `cargo install cargo-ndk` and `patchelf`.

```sh
./scripts/build.sh --ios          # dist/ios/: libbmp_steam.dylib, libssl.3.dylib, libcrypto.3.dylib
./scripts/build.sh --android      # dist/android-arm64/: libbmp_steam.so, libbmp_ssl.so, libbmp_crypto.so
./scripts/build.sh --all
./scripts/package.sh              # dist/release/: MultiplayerMobile-<v>.zip (mod + Android libraries),
                                  #   BMP-Mobile-iOS-<v>.zip (tweak folder), SHA256SUMS
python3 -I openssl/check-symbols.py <MultiplayerAPI>/networking/openssl_ffi.lua nm dist/ios/libssl.3.dylib dist/ios/libcrypto.3.dylib
```

**Mod tests** (from `mod/`): `luajit tests/mobile_test.lua`, `stylua --check main.lua src localization tests`.

**What the mod needs from MultiplayerAPI:** its Steam provider hooks
(`MPAPI.networking.steam.register_provider`, `provider_changed`), `MPAPI.add_account_overlay_button`,
`MPAPI.text_input_active`, and the `BET_OPENSSL_SSL_PATH` / `BET_OPENSSL_CRYPTO_PATH` overrides.
Without them the mod warns and does nothing.

**Desktop tests and harness:**
```sh
cargo test
cargo run -p bridge-cli -- login --qr
cargo run -p bridge-cli -- ticket
STEAM_WEB_API_KEY=... cargo run -p bridge-cli -- verify   # checks a ticket like the BMP server does
cargo run -p bridge-cli -- nakama-login --host 127.0.0.1 --port 7350   # a local BMP server only
```
The harness refuses `*.balatromp.com`.

## License

MIT (see `LICENSE`). Third-party notices are in `NOTICE`. No SteamKit2 (LGPL) code is used.
