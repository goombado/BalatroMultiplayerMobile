# Vendored crates

## steam-vent 0.5.0 (MIT, https://codeberg.org/steam-vent/steam-vent)

Copied from the crates.io release. One change:

- **`src/connection/unauthenticated.rs`:** adds the public
  `UnAuthenticatedConnection::service_method`, a wrapper around the crate-private
  `service_method_un_authenticated`. The bridge needs it to run
  `IAuthenticationService` QR and Steam Guard flows over the CM socket before logon,
  the way Steam's SteamClient-platform login (and node-steam-session) does.

If upstream exposes this, drop the vendored copy and the `[patch]` entry.
