//! Where the Steam refresh token lives between launches.
//!
//! - iOS: the Keychain (service `com.bmp.steambridge`). If the Keychain is unavailable (some
//!   sideload/LiveContainer setups have no usable access group), fall back to the file below.
//! - Android and desktop: `steam_login.json` (mode 0600) in the data dir passed to
//!   `bmp_bridge_init`. On Android that is the app-private save dir, so it is protected only by the
//!   app sandbox.
//!
//! Only the refresh token is secret. The account name is needed for logon, and the guard data lets a
//! Steam Guard email account skip the code next time.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LOGIN_FILE_NAME: &str = "steam_login.json";

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredLogin {
    pub account_name: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_data: Option<String>,
}

impl std::fmt::Debug for StoredLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredLogin").field("account_name", &self.account_name).finish_non_exhaustive()
    }
}

/// The JWT claims the bridge reads. Not verified: Steam verifies the token, we only schedule renewal.
#[derive(Debug, Deserialize)]
pub struct TokenClaims {
    pub sub: String,
    pub exp: u64,
}

pub fn decode_claims(jwt: &str) -> Option<TokenClaims> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Store { dir: dir.to_owned() }
    }

    fn file(&self) -> PathBuf {
        self.dir.join(LOGIN_FILE_NAME)
    }

    pub fn load(&self) -> Option<StoredLogin> {
        let login = platform::load().or_else(|| {
            let bytes = std::fs::read(self.file()).ok()?;
            serde_json::from_slice::<StoredLogin>(&bytes).ok()
        })?;
        crate::redact::register(&login.refresh_token);
        Some(login)
    }

    pub fn save(&self, login: &StoredLogin) -> std::io::Result<()> {
        crate::redact::register(&login.refresh_token);
        let json = serde_json::to_vec(login).map_err(std::io::Error::other)?;
        if platform::save(&json) {
            // Don't leave an older copy behind in the file fallback.
            let _ = std::fs::remove_file(self.file());
            return Ok(());
        }
        write_private(&self.file(), &json)
    }

    pub fn delete(&self) {
        platform::delete();
        let _ = std::fs::remove_file(self.file());
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(target_os = "ios")]
mod platform {
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    const SERVICE: &str = "com.bmp.steambridge";
    const ACCOUNT: &str = "steam-login";

    pub fn load() -> Option<super::StoredLogin> {
        let bytes = get_generic_password(SERVICE, ACCOUNT).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save(json: &[u8]) -> bool {
        match set_generic_password(SERVICE, ACCOUNT, json) {
            Ok(()) => true,
            Err(e) => {
                crate::log::log!("keychain save failed ({e}); using the file in the data dir");
                false
            }
        }
    }

    pub fn delete() {
        let _ = delete_generic_password(SERVICE, ACCOUNT);
    }
}

#[cfg(not(target_os = "ios"))]
mod platform {
    pub fn load() -> Option<super::StoredLogin> {
        None
    }
    pub fn save(_json: &[u8]) -> bool {
        false
    }
    pub fn delete() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip_is_private_and_deletable() {
        let dir = std::env::temp_dir().join(format!("bmp-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(&dir);
        let login = StoredLogin { account_name: "acct".into(), refresh_token: "eyJx.eyJy.sig-secret".into(), guard_data: None };
        store.save(&login).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(LOGIN_FILE_NAME)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let back = store.load().unwrap();
        assert_eq!(back.account_name, "acct");
        assert_eq!(back.refresh_token, login.refresh_token);
        assert!(!format!("{back:?}").contains("sig-secret"), "Debug must not print the token");

        store.delete();
        assert!(store.load().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claims_decode() {
        // {"sub":"76561198000000000","exp":1900000000,"aud":["client","derive"]}
        let payload = "eyJzdWIiOiI3NjU2MTE5ODAwMDAwMDAwMCIsImV4cCI6MTkwMDAwMDAwMCwiYXVkIjpbImNsaWVudCIsImRlcml2ZSJdfQ";
        let c = decode_claims(&format!("eyJhbGciOiJFZERTQSJ9.{payload}.sig")).unwrap();
        assert_eq!(c.sub, "76561198000000000");
        assert_eq!(c.exp, 1900000000);
        assert!(decode_claims("not-a-jwt").is_none());
    }
}
