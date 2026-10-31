# Device test checklist (Phase 5)

Run against a **local** server first: `BalatroMultiplayerServer`, `docker compose -f compose.local.yml up -d`,
with `STEAM_WEB_API_KEY` and `PLAYER_ID_SALT` set in `.env`.

- To reach it from the phone, set `NAKAMA_BIND_ADDRESS=0.0.0.0`.
- In MultiplayerAPI's config set `use_custom_server`, `server_host=<PC LAN IP>`, `server_port=7350`
  and `server_secure=false`.
- To force the Steam-ticket login (not device login) on a non-official server, use the Dev mod's
  `force_steam_login`, or `BMP_SERVER_KIND=dev`.

Send back for each step: pass/fail, a screenshot of anything odd, MultiplayerAPI's log and
`bmp_steam_bridge.log` from the save folder. Read the bridge log first: it has no secrets, but it does
show your login state.

## 0. Desktop regression (PC, before any phone test)
- [ ] Desktop Balatro with the modified MultiplayerAPI logs in and reaches **Connected** exactly as before.
- [ ] PvP and Speedrun menus load; a private lobby works.

## 1. Fresh install
- [ ] The Multiplayer panel shows **Log in to Steam** / **Offline / Tap to log in to Steam**, not a hang.
- [ ] PvP/Speedrun still load (offline play works).
- [ ] Account name + password: the phone keyboard types lowercase; the password shows only the last
      typed character; approving in the Steam app logs in and the overlay closes itself.
- [ ] Denying the sign-in in the Steam app returns to the login form with an error.
- [ ] Steam Guard code instead of approval (app code, or email code on an account without the app).
- [ ] **Show QR code for another device**: scanning with the Steam app on another device logs in,
      also after leaving it unscanned for over a minute (Steam drops idle connections).

## 2. Connection (local server, plain WS)
- [ ] After login the panel goes Connecting → Signing In → **Connected** without pressing anything.
- [ ] The server log shows a custom (Steam) login, not a device login.

## 3. TLS
- [ ] Against a TLS endpoint (the local server behind `spikes/local-tls-proxy`, or the official
      server once the maintainers OK it), the client connects. This proves the mobile OpenSSL works.

## 4. Gameplay
- [ ] Private PvP lobby, phone vs desktop: full game.
- [ ] A Speedrun lobby.
- [ ] Background the app for 1 minute, return: it reconnects.

## 5. Restart
- [ ] Kill and relaunch: it logs in by itself (stored token) and connects.
- [ ] Account overlay → **Log out of Steam**: the panel shows **Log in to Steam**; relaunch: it asks again.
- [ ] Log out, then log in again without restarting: the panel reaches **Connected** without Retry.
- [ ] Desktop: the account overlay has no **Log out of Steam** button.

## 6. Error paths
- [ ] No tweak installed (iOS): **Steam bridge not installed**.
- [ ] Account without Balatro (if available): **Steam account lacks Balatro**.
- [ ] Airplane mode at launch, then back online: the bridge reconnects to Steam.

## 7. Android (emulator is fine: arm64 system image)
- [ ] First launch stages `MPAPI_native/<version>/` in the save folder; the log shows "Staged the Android libraries".
- [ ] OpenSSL and the bridge load from there; steps 1–2 pass.
- [ ] Android 14+ image.
