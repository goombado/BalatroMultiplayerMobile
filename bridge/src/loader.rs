//! Load-time constructor. Kept minimal: no networking, no allocation-heavy work, no logging.
//!
//! On iOS the dylib is injected (LiveContainer tweak folder or the app's Frameworks/) next to
//! libssl.3.dylib and libcrypto.3.dylib. Exporting that folder as BET_OPENSSL_DYLIB_DIR lets
//! MultiplayerAPI's existing macOS loader in openssl_ffi.lua find them without new code. It does
//! not overwrite a value that is already set. BMP_STEAM_BRIDGE=1 tells Lua the bridge is in the
//! process. On Android, MultiplayerAPI stages and loads everything itself, so nothing is set there.

use std::sync::OnceLock;

static LIB_DIR: OnceLock<String> = OnceLock::new();

pub fn openssl_dir() -> &'static str {
    if cfg!(target_os = "ios") {
        LIB_DIR.get().map(String::as_str).unwrap_or("")
    } else {
        ""
    }
}

#[cfg(unix)]
fn own_dir() -> Option<String> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    let addr = crate::ffi::bmp_bridge_version as *const libc::c_void;
    if unsafe { libc::dladdr(addr, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    let path = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }.to_str().ok()?;
    std::path::Path::new(path).parent().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(target_os = "ios")]
#[ctor::ctor]
fn on_load() {
    let Some(dir) = own_dir() else { return };
    if let Ok(c) = std::ffi::CString::new(dir.clone()) {
        unsafe {
            libc::setenv(c"BET_OPENSSL_DYLIB_DIR".as_ptr(), c.as_ptr(), 0);
            libc::setenv(c"BMP_STEAM_BRIDGE".as_ptr(), c"1".as_ptr(), 1);
        }
    }
    let _ = LIB_DIR.set(dir);
}

/// Exposed for the desktop CLI's self-check; not part of the C ABI.
#[cfg(unix)]
pub fn library_dir() -> Option<String> {
    own_dir()
}
