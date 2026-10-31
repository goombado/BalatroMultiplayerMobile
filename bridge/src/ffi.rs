//! The C ABI. Every export is thread-safe, returns within microseconds (a short lock at most), and
//! never calls back into the caller: LuaJIT FFI callbacks are unreliable without JIT on iOS.
//!
//! Returned strings are UTF-8, NUL-terminated, and owned by the bridge. Each one stays valid until
//! the next call to the same function on the same thread.

use crate::state::{self, BridgeState, Command};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};

pub const ABI_VERSION: c_int = 1;

thread_local! {
    static RETURNED: RefCell<HashMap<&'static str, CString>> = RefCell::new(HashMap::new());
}

fn ret_str(slot: &'static str, s: &str) -> *const c_char {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    RETURNED.with(|r| {
        let mut r = r.borrow_mut();
        r.insert(slot, c);
        r[slot].as_ptr()
    })
}

unsafe fn arg_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok().map(str::to_owned)
}

// ---- Steamworks-compatible exports ------------------------------------------------------------
// These are what MultiplayerAPI's networking/steam.lua already resolves from libsteam_api. The
// ISteamUser "pointer" is a sentinel: our functions ignore `self`.

static STEAM_USER_SENTINEL: u8 = 0;

fn steam_user() -> *mut c_void {
    if state::shared().state == BridgeState::Ready {
        &STEAM_USER_SENTINEL as *const u8 as *mut c_void
    } else {
        std::ptr::null_mut()
    }
}

/// Non-NULL once logged in with a ticket ready, else NULL.
#[no_mangle]
pub extern "C" fn SteamAPI_SteamUser_v023() -> *mut c_void {
    steam_user()
}
#[no_mangle]
pub extern "C" fn SteamAPI_SteamUser_v022() -> *mut c_void {
    steam_user()
}
#[no_mangle]
pub extern "C" fn SteamAPI_SteamUser_v021() -> *mut c_void {
    steam_user()
}
#[no_mangle]
pub extern "C" fn SteamAPI_SteamUser_v020() -> *mut c_void {
    steam_user()
}

/// Copies a pre-activated Balatro auth session ticket into `buf` and returns its handle, or 0 if
/// none is ready (or `max` is too small). `identity` is ignored, as with a NULL identity in Steamworks.
///
/// # Safety
/// `buf` must have room for `max` bytes and `out_len` must be valid for a write (or NULL).
#[no_mangle]
pub unsafe extern "C" fn SteamAPI_ISteamUser_GetAuthSessionTicket(
    _self: *mut c_void,
    buf: *mut c_void,
    max: c_int,
    out_len: *mut u32,
    _identity: *const c_void,
) -> u32 {
    if buf.is_null() || max <= 0 {
        return 0;
    }
    let ticket = {
        let mut s = state::shared();
        if s.state != BridgeState::Ready {
            return 0;
        }
        match s.ticket_pool.front() {
            Some(t) if t.bytes.len() <= max as usize => s.ticket_pool.pop_front(),
            _ => None,
        }
    };
    let Some(t) = ticket else { return 0 };
    std::ptr::copy_nonoverlapping(t.bytes.as_ptr(), buf as *mut u8, t.bytes.len());
    if !out_len.is_null() {
        *out_len = t.bytes.len() as u32;
    }
    state::send(Command::Refill);
    t.handle
}

#[no_mangle]
pub extern "C" fn SteamAPI_ISteamUser_CancelAuthTicket(_self: *mut c_void, handle: u32) {
    if handle != 0 {
        state::send(Command::CancelTicket(handle));
    }
}

// ---- Bridge API -------------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn bmp_bridge_version() -> c_int {
    ABI_VERSION
}

/// Idempotent. Starts the worker and logs in automatically if a token is stored. `data_dir` must
/// be a writable app-private directory (the game's save dir).
///
/// # Safety
/// `data_dir` must be NULL or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn bmp_bridge_init(data_dir: *const c_char) {
    let Some(dir) = arg_str(data_dir).filter(|d| !d.is_empty()) else {
        state::set_error("bmp_bridge_init needs a data directory");
        state::set_state(BridgeState::Error);
        return;
    };
    crate::init(std::path::PathBuf::from(dir));
}

#[no_mangle]
pub extern "C" fn bmp_bridge_state() -> c_int {
    state::shared().state as c_int
}

#[no_mangle]
pub extern "C" fn bmp_bridge_last_error() -> *const c_char {
    let e = state::shared().last_error.clone();
    ret_str("last_error", &e)
}

#[no_mangle]
pub extern "C" fn bmp_bridge_qr_url() -> *const c_char {
    let u = state::shared().qr_url.clone();
    ret_str("qr_url", &u)
}

/// "app" (authenticator code), "email", or "" when no code is being asked for.
#[no_mangle]
pub extern "C" fn bmp_bridge_guard_kind() -> *const c_char {
    let k = state::shared().guard_kind.clone();
    ret_str("guard_kind", &k)
}

#[no_mangle]
pub extern "C" fn bmp_bridge_begin_qr_login() {
    state::send(Command::BeginQr);
}

/// # Safety
/// Both arguments must be NULL or NUL-terminated strings. The bridge copies them; the caller should
/// clear its own copy right after.
#[no_mangle]
pub unsafe extern "C" fn bmp_bridge_begin_password_login(user: *const c_char, pass: *const c_char) {
    let (Some(user), Some(pass)) = (arg_str(user), arg_str(pass)) else { return };
    crate::redact::register(&pass);
    state::send(Command::BeginPassword { user, pass });
}

/// # Safety
/// `code` must be NULL or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn bmp_bridge_submit_guard_code(code: *const c_char) {
    if let Some(code) = arg_str(code) {
        crate::redact::register(&code);
        state::send(Command::GuardCode(code));
    }
}

#[no_mangle]
pub extern "C" fn bmp_bridge_cancel_login() {
    state::send(Command::CancelLogin);
}

/// Ends the session and deletes the stored token.
#[no_mangle]
pub extern "C" fn bmp_bridge_logout() {
    state::send(Command::Logout);
}

/// SteamID64 as a decimal string, "" if not logged in.
#[no_mangle]
pub extern "C" fn bmp_bridge_steam_id() -> *const c_char {
    let id = state::shared().steam_id;
    ret_str("steam_id", &if id == 0 { String::new() } else { id.to_string() })
}

#[no_mangle]
pub extern "C" fn bmp_bridge_persona_name() -> *const c_char {
    let n = state::shared().persona_name.clone();
    ret_str("persona_name", &n)
}

/// iOS: the folder this library was loaded from (where the OpenSSL dylibs are expected). Else "".
#[no_mangle]
pub extern "C" fn bmp_bridge_openssl_dir() -> *const c_char {
    ret_str("openssl_dir", crate::loader::openssl_dir())
}
