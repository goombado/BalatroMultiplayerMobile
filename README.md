# Multiplayer Mobile

Lets Balatro builds made with the Lovely mobile maker (iOS and Android) log in to Balatro Multiplayer's
**unchanged** official server with your own Steam account. Two parts:

- **The mod** (`mod/`, SMODS id `MultiplayerMobile`): installed next to MultiplayerAPI. It registers the
  Steam bridge as MultiplayerAPI's Steam provider and adds the Steam login screen. It does nothing on
  desktop.
- **The Steam bridge** (`bridge/`, Rust): a native library that logs in to Steam and mints tickets.

On desktop, MultiplayerAPI gets a Steam auth session ticket from the Steam client and the server checks it
with Valve (`ISteamUserAuth/AuthenticateUserTicket`). Phones have no Steam client, so the bridge does the
client's part itself:

1. It logs in to Steam over the Steam client protocol (CM WebSocket).
2. It mints a genuine Balatro (app 2379780) auth session ticket and registers it with Steam.
3. It hands the ticket out through the same Steamworks C functions MultiplayerAPI already looks up.

The server is not changed and can't tell the difference: the ticket is real, and it is checked the same way.
The Steam account must own Balatro. Ranked is out of scope: it needs the desktop anti-cheat launcher.

## Using it

Install steps are in [docs/INSTALL.md](docs/INSTALL.md). In short:

- **Android:** unzip `MultiplayerMobile-<version>.zip` from a release into `Mods/`. It carries the
  Android libraries.
- **iOS:** the same mod zip, plus `BMP-Mobile-iOS-<version>.zip` as a LiveContainer tweak folder (or
  injected with Sideloadly). iOS only loads libraries signed with the app, so they can't come from the
  mod.
- **MultiplayerAPI** must have the Steam provider hooks. They are not in an official release yet; they
  are on the `feature/steam-provider-hooks` branch of
  [goombado/BalatroMultiplayerAPI](https://github.com/goombado/BalatroMultiplayerAPI/tree/feature/steam-provider-hooks).
  Without them the mod logs a warning and does nothing.

In the Multiplayer panel, a phone shows **Log in to Steam**. The login screen offers:

- **Account name and password** (the default), typed with the phone's own keyboard. The password shows
  only the last character typed. Steam then asks you to approve the sign-in in the Steam app, or for a
  Steam Guard code (from the app, or emailed).
- **Show QR code for another device**: scan it with the Steam app on a different device.

Once logged in, the game connects by itself, and logs in again by itself on later launches. To switch
accounts, open your Multiplayer account and tap **Log out of Steam**.

## Security model

- **Credentials:** your Steam password, refresh token and access tokens only ever go to Valve
  (`*.steamserver.net` CM sockets, `api.steampowered.com`). Balatro Multiplayer never sees them. The
  mod hands the password straight to the bridge and drops its own copy.
- **Stored login:** a SteamClient refresh token (about 200 days; renewed when under 30 days are left).
  - iOS: the Keychain. It falls back to a 0600 file in the save dir if the Keychain is unavailable.
  - Android: a 0600 file in the app-private save dir, protected by the app sandbox only.
- **Logout:** logging out deletes the token.
- **Redaction:** every secret is registered with a redactor, and everything written to the log
  (`bmp_steam_bridge.log` in the save dir) or returned as an error passes through it. JWT-shaped
  strings are scrubbed too.
- **What Steam sees:** a Steam client session for your account, named
  "Balatro Multiplayer (iOS)" or "(Android)" in Steam's authorized devices. It does not show you as
  in-game; tickets validate without that. (`BMP_BRIDGE_GAMES_PLAYED=1` turns it on, for testing.)
- **What Balatro Multiplayer sees:** MultiplayerAPI's usual login and mod list. Multiplayer Mobile shows
  in that list like any other mod; nothing MultiplayerAPI reports is hidden or altered.

## How the mod plugs in

Everything goes through MultiplayerAPI's public hooks; the mod patches none of its files.

- `MPAPI.networking.steam.register_provider{...}` makes the bridge the source of the Steam id, persona
  name and tickets when there is no Steam client. When `G.STEAM` exists, MultiplayerAPI ignores the
  provider. `provider_changed()` tells MultiplayerAPI the bridge's state moved on: the menu refreshes,
  and logging in connects.
- The provider's `login_needed` / `login_label` / `open_login` turn the account button into
  **Log in to Steam**, which opens this mod's login screen.
- `MPAPI.add_account_overlay_button` adds **Log out of Steam** to the account overlay.
- `MPAPI.text_input_active` keeps MultiplayerAPI's chat key (`t`) out of the login fields.
- Android: the mod copies its libraries into `<save dir>/MPM_native/<version>/` (the dynamic linker
  can't load from a mod folder), then sets `BET_OPENSSL_SSL_PATH` / `BET_OPENSSL_CRYPTO_PATH` for
  MultiplayerAPI's OpenSSL loader.
- iOS: the bridge's load-time constructor exports its own folder as `BET_OPENSSL_DYLIB_DIR` (unless
  already set), so MultiplayerAPI's existing macOS OpenSSL loader finds `libssl.3.dylib` /
  `libcrypto.3.dylib` injected next to it.

## Bridge ABI

See `include/bmp_steam_bridge.h` (generated, ABI version 1).

- **Steamworks-compatible exports:** `SteamAPI_SteamUser_v020`…`v023`,
  `SteamAPI_ISteamUser_GetAuthSessionTicket` and `SteamAPI_ISteamUser_CancelAuthTicket`.
- **Login API:** `bmp_bridge_*` (password and Steam Guard, QR, cancel, logout, state, persona name),
  polled; there are no callbacks.
- **Speed:** every call returns in microseconds. A pool of two pre-activated tickets lets
  `GetAuthSessionTicket` answer synchronously, as Steamworks does.

## Layout

| Path | What |
|---|---|
| `mod/` | The SMODS mod (Lua): `main.lua`, `src/`, `localization/`, `lib/qrencode/` (vendored), `tests/` |
| `mod/native/` | Empty in the repository; `scripts/package.sh` puts the Android libraries here |
| `bridge/` | The library (`cdylib` + `rlib`, crate `bmp-steam-bridge`, lib name `bmp_steam`) |
| `bridge-cli/` | Desktop harness that drives the same C ABI |
| `include/` | The generated C header |
| `vendor/steam-vent/` | steam-vent 0.5.0 with one patch (`vendor/PATCHES.md`) |
| `openssl/` | Pinned OpenSSL 3.6.5 builds for iOS/Android and a symbol check against MultiplayerAPI's cdefs |
| `scripts/` | `build.sh --ios / --android / --all`, `package.sh` (release zips), `gen-header.sh` |
| `docs/` | Install guide, device test checklist |

## Building a release

The repository holds no binaries: build them locally and attach the `scripts/package.sh` output to a
GitHub release. You need Rust (see `rust-toolchain.toml`), Xcode for iOS, and for Android the NDK
(`ANDROID_NDK_HOME`), `cargo install cargo-ndk` and `patchelf`.

```sh
./scripts/build.sh --all          # or --ios / --android
                                  #   dist/ios/: libbmp_steam.dylib, libssl.3.dylib, libcrypto.3.dylib
                                  #   dist/android-arm64/: libbmp_steam.so, libbmp_ssl.so, libbmp_crypto.so
./scripts/package.sh              # dist/release/: MultiplayerMobile-<v>.zip (mod + Android libraries),
                                  #   BMP-Mobile-iOS-<v>.zip (tweak folder), SHA256SUMS
```

The version comes from `mod/MultiplayerMobile.json`. Before releasing, check that OpenSSL exports
everything MultiplayerAPI declares:

```sh
python3 -I openssl/check-symbols.py <MultiplayerAPI>/networking/openssl_ffi.lua nm dist/ios/libssl.3.dylib dist/ios/libcrypto.3.dylib
python3 -I openssl/check-symbols.py <MultiplayerAPI>/networking/openssl_ffi.lua "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-nm dist/android-arm64/libbmp_ssl.so dist/android-arm64/libbmp_crypto.so
```

After changing the bridge's C API, regenerate the header with `./scripts/gen-header.sh`.

## Tests

```sh
cargo test                                  # bridge
cd mod && luajit tests/mobile_test.lua      # mod
stylua --check main.lua src localization tests
```

The desktop harness drives the same C ABI as the game:

```sh
cargo run -p bridge-cli -- login --qr                    # or --password
cargo run -p bridge-cli -- status
cargo run -p bridge-cli -- ticket
STEAM_WEB_API_KEY=... cargo run -p bridge-cli -- verify  # checks a ticket like the BMP server does
cargo run -p bridge-cli -- nakama-login --host 127.0.0.1 --port 7350   # a local BMP server only
cargo run -p bridge-cli -- logout
```

The harness refuses `*.balatromp.com`. `docs/TEST_CHECKLIST.md` lists the on-device tests.

## License

MIT (see `LICENSE`). Third-party notices are in `NOTICE`. No SteamKit2 (LGPL) code is used.
