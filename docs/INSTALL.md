# Playing Balatro Multiplayer on iOS / Android

You need:
- A Balatro build made with the Lovely mobile maker, with Steamodded installed.
- MultiplayerAPI (a version with Steam provider support) plus MultiplayerPvP and/or MultiplayerSpeedrunning.
- **Multiplayer Mobile**: `MultiplayerMobile-<version>.zip` from this project's release. Unzip it into
  `Mods/` like any other mod. It does nothing on desktop.
- A Steam account that **owns Balatro on Steam**. The login is checked against your Steam licence.
- Ideally the Steam mobile app, for QR login and Steam Guard.

Ranked is not available on mobile: it needs the desktop launcher. Casual matchmaking and private lobbies work.

## iOS with LiveContainer

1. Install your Balatro build and the mods as usual.
2. Import the IPA into LiveContainer.
3. Get `BMP-Mobile-iOS-<version>.zip` from the release and unzip it: a `BMP-Mobile` folder with
   `libbmp_steam.dylib`, `libssl.3.dylib`, `libcrypto.3.dylib` and their `SHA256SUMS`.
4. LiveContainer → **Tweaks** → import the `BMP-Mobile` folder (or create it and import the three dylibs).
5. LiveContainer → Balatro's app settings → **Tweak folder** → `BMP-Mobile`.
6. Launch Balatro. In the Multiplayer panel tap **Log in to Steam** and either:
   - type your Steam account name and password, then approve the sign-in in the Steam app (or type the
     Steam Guard code it shows, or the one Steam emails you), or
   - tap **Show QR code for another device** and scan it with the Steam app on another device.

Your login is kept until you log out: Multiplayer panel → your account → **Log out of Steam**. Log out
there to switch to another Steam account.

## iOS with Sideloadly

Same as above, but inject the three dylibs with **Advanced options → Inject dylibs/frameworks** when you
sign the IPA, instead of steps 2, 4 and 5.

## Android

Nothing extra: the Multiplayer Mobile mod carries the Android libraries (`native/android-arm64/`) and
copies them into the app's private storage on first launch. Then log in as in step 6 above.

## Troubleshooting

The reason is shown under **Offline** in the Multiplayer panel.

| Message | What to do |
|---|---|
| Steam bridge not installed | iOS: the tweak folder isn't set, or the dylibs are missing from it. Check steps 3–5. |
| Tap to log in to Steam | Not logged in yet, or Steam ended the saved login. Tap **Log in to Steam** and log in. |
| Steam account lacks Balatro | That Steam account doesn't own Balatro. Log out and use the one that does. |
| Native libraries failed | Android: the save folder is not in internal storage, or copying failed. Reinstall the mod. |
| OpenSSL not found | iOS: `libssl.3.dylib`/`libcrypto.3.dylib` are missing from the tweak folder. Android: reinstall the mod. |

- **Playing on PC and phone with the same Steam account:** both stay logged in. If Steam reports the
  account as logged in elsewhere, the phone waits a few minutes before reconnecting.
- **Logs:** MultiplayerAPI's debug log, plus `bmp_steam_bridge.log` in the game's save folder. The bridge
  log never contains passwords or tokens, but it does show your Steam login state, so look it over
  before sharing it.
