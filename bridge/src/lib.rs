//! bmp-steam-bridge: logs into Steam over the CM WebSocket protocol and mints genuine Balatro
//! (app 2379780) auth session tickets. It exposes them through the Steamworks C symbols that
//! Balatro Multiplayer's MultiplayerAPI already resolves, plus a small `bmp_bridge_*` API for login.
//! See `include/bmp_steam_bridge.h` and the README.

mod auth;
pub mod ffi;
pub mod loader;
mod log;
#[doc(hidden)]
pub mod net;
pub mod redact;
mod session;
mod state;
mod store;
pub mod ticket;
mod worker;

pub const BALATRO_APP_ID: u32 = 2379780;

fn init(data_dir: std::path::PathBuf) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::fs::create_dir_all(&data_dir);
        log::init(&data_dir);
        log::log!("bmp-steam-bridge {} starting (ABI {})", env!("CARGO_PKG_VERSION"), ffi::ABI_VERSION);
        state::set_state(state::BridgeState::Connecting);
        if let Err(e) = worker::start(data_dir) {
            state::set_error(&format!("could not start the bridge worker: {e}"));
            state::set_state(state::BridgeState::Error);
        }
    });
}

/// For the desktop CLI: mirror the bridge log to stderr.
pub fn echo_log_to_stderr(on: bool) {
    log::echo_to_stderr(on);
}
