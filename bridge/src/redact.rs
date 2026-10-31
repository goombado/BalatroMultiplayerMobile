//! Keeps secrets out of logs and error strings.
//!
//! Every password, refresh token, access token and Steam Guard code the bridge handles is
//! registered here, and everything written to the log or returned through `bmp_bridge_last_error`
//! passes through `redact`. Anything shaped like a JWT is scrubbed as well, in case a token we never
//! registered (e.g. a renewed one inside an error message) slips through.

use std::sync::Mutex;

static SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Too-short values would blank out ordinary text, and a 1–3 character secret can't usefully be hidden anyway.
const MIN_SECRET_LEN: usize = 4;

pub fn register(secret: &str) {
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    let mut s = SECRETS.lock().unwrap_or_else(|e| e.into_inner());
    if !s.iter().any(|x| x == secret) {
        s.push(secret.to_owned());
    }
}

pub fn redact(text: &str) -> String {
    let mut out = text.to_owned();
    {
        let s = SECRETS.lock().unwrap_or_else(|e| e.into_inner());
        for secret in s.iter() {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), "<redacted>");
            }
        }
    }
    scrub_jwts(&out)
}

fn is_b64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

// A JWT is three base64url segments joined by dots, and its header always starts with "eyJ" ('{"').
fn scrub_jwts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("eyJ") {
        out.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let end = candidate.find(|c: char| !(is_b64url(c) || c == '.')).unwrap_or(candidate.len());
        let token = &candidate[..end];
        if token.split('.').filter(|p| !p.is_empty()).count() >= 3 {
            out.push_str("<jwt>");
        } else {
            out.push_str(token);
        }
        rest = &candidate[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_secrets_are_removed() {
        register("hunter2-password");
        register("abc"); // too short to register
        let r = redact("login with hunter2-password failed (abc)");
        assert!(!r.contains("hunter2-password"));
        assert!(r.contains("<redacted>"));
        assert!(r.contains("abc"));
    }

    #[test]
    fn jwts_are_scrubbed_even_unregistered() {
        let jwt = "eyJhbGciOiJFZERTQSJ9.eyJzdWIiOiI3NjU2MTE5OCJ9.c2lnbmF0dXJl";
        let r = redact(&format!("token={jwt}; next"));
        assert_eq!(r, "token=<jwt>; next");
    }

    #[test]
    fn non_jwt_text_is_kept() {
        assert_eq!(redact("eyJ alone and a.b.c"), "eyJ alone and a.b.c");
    }
}
