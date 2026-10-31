//! Getting a SteamClient-platform refresh token: QR login, or password + Steam Guard.
//!
//! The IAuthenticationService calls go over an unauthenticated CM WebSocket, as Steam's own client
//! and node-steam-session do for this platform. CM logon only accepts SteamClient-platform refresh
//! tokens. The device details match node-steam-session's SteamClient defaults, which spike 0.1
//! proved, except that the friendly name says what the device really is.
//!
//! The flows poll and react to `Command`s from the C ABI (guard code, cancel, or switching from QR
//! to password), so the ABI never blocks.

use crate::log::log;
use crate::net;
use crate::redact;
use crate::state::{self, BridgeState, Command};
use crate::store::StoredLogin;
use base64::Engine;
use std::time::{Duration, Instant};
use steam_vent::connection::UnAuthenticatedConnection;
use steam_vent::{EResult, NetworkError};
use steam_vent_proto_common::protobuf::{EnumOrUnknown, MessageField};
use steam_vent_proto_steam::enums::ESessionPersistence;
use steam_vent_proto_steam::steammessages_auth_steamclient::*;
use tokio::sync::mpsc::UnboundedReceiver;

const QR_LOGIN_DEADLINE: Duration = Duration::from_secs(10 * 60);
const PASSWORD_LOGIN_DEADLINE: Duration = Duration::from_secs(5 * 60);
const EOS_TYPE_WIN11: i32 = 20;

pub enum AuthOutcome {
    LoggedIn(StoredLogin),
    Cancelled,
    /// Another login command arrived mid-flow; the worker runs it next.
    Switch(Command),
    Failed(String),
}

fn device_name() -> String {
    let device = if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "android") {
        "Android"
    } else {
        "desktop test"
    };
    format!("Balatro Multiplayer ({device})")
}

fn device_details() -> CAuthentication_DeviceDetails {
    CAuthentication_DeviceDetails {
        device_friendly_name: Some(device_name()),
        platform_type: Some(EnumOrUnknown::new(EAuthTokenPlatformType::k_EAuthTokenPlatformType_SteamClient)),
        os_type: Some(EOS_TYPE_WIN11),
        gaming_device_type: Some(1),
        ..Default::default()
    }
}

pub async fn connect_unauthenticated() -> Result<UnAuthenticatedConnection, String> {
    let servers = net::cm_servers().await?;
    let mut last = String::new();
    for endpoint in servers.iter().take(3) {
        match net::connect_cm(endpoint).await {
            Ok((sink, stream, _closed)) => match UnAuthenticatedConnection::from_sender_receiver(sink, stream).await {
                Ok(c) => return Ok(c),
                Err(e) => last = format!("hello to {endpoint}: {e}"),
            },
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Steam drops an idle unauthenticated CM socket after about a minute, well inside a QR or Steam Guard
/// wait. The auth session lives on Steam's side, so a fresh connection carries on polling it.
const MAX_RECONNECTS: u32 = 5;

/// True when `e` is the socket failing rather than Steam answering with an error.
fn is_transport(e: &NetworkError) -> bool {
    api_error(e).is_none()
}

async fn reconnect(conn: &mut UnAuthenticatedConnection, reconnects: &mut u32, why: &NetworkError) -> Result<(), String> {
    if *reconnects >= MAX_RECONNECTS {
        return Err(format!("lost the connection to Steam: {why}"));
    }
    *reconnects += 1;
    log!("Steam connection dropped ({why}); reconnecting");
    *conn = connect_unauthenticated().await.map_err(|e| format!("Could not reach Steam: {e}"))?;
    Ok(())
}

fn api_error(e: &NetworkError) -> Option<EResult> {
    match e {
        NetworkError::ApiError(r) => Some(*r),
        _ => None,
    }
}

/// What a command means to a flow that is waiting on Steam.
enum Interrupt {
    Cancel,
    Switch(Command),
    Code(String),
    Ignore,
}

fn classify(cmd: Option<Command>) -> Interrupt {
    match cmd {
        None | Some(Command::CancelLogin) | Some(Command::Logout) => Interrupt::Cancel,
        Some(c @ Command::BeginQr) | Some(c @ Command::BeginPassword { .. }) => Interrupt::Switch(c),
        Some(Command::GuardCode(code)) => Interrupt::Code(code),
        Some(_) => Interrupt::Ignore,
    }
}

fn interval_of(secs: Option<f32>) -> Duration {
    Duration::from_secs_f32(secs.unwrap_or(5.0).clamp(1.0, 10.0))
}

pub async fn qr_login(rx: &mut UnboundedReceiver<Command>) -> AuthOutcome {
    state::set_state(BridgeState::Authenticating);
    let mut conn = match connect_unauthenticated().await {
        Ok(c) => c,
        Err(e) => return AuthOutcome::Failed(format!("Could not reach Steam: {e}")),
    };
    let mut reconnects = 0;
    let deadline = Instant::now() + QR_LOGIN_DEADLINE;

    // An expired QR code is replaced with a fresh one until the overall deadline.
    'new_code: while Instant::now() < deadline {
        let begin = conn
            .service_method(CAuthentication_BeginAuthSessionViaQR_Request {
                device_friendly_name: Some(device_name()),
                platform_type: Some(EnumOrUnknown::new(EAuthTokenPlatformType::k_EAuthTokenPlatformType_SteamClient)),
                device_details: MessageField::some(device_details()),
                website_id: Some("Client".into()),
                ..Default::default()
            })
            .await;
        let begin = match begin {
            Ok(b) => b,
            Err(e) if is_transport(&e) => match reconnect(&mut conn, &mut reconnects, &e).await {
                Ok(()) => continue 'new_code,
                Err(msg) => return AuthOutcome::Failed(msg),
            },
            Err(e) => return AuthOutcome::Failed(format!("Steam refused to start a QR login: {e}")),
        };
        let mut client_id = begin.client_id.unwrap_or_default();
        let request_id = begin.request_id.clone().unwrap_or_default();
        let interval = interval_of(begin.interval);
        state::shared().qr_url = begin.challenge_url.clone().unwrap_or_default();
        state::set_state(BridgeState::AwaitingQr);
        state::clear_error();
        log!("QR login started");

        loop {
            tokio::select! {
                cmd = rx.recv() => match classify(cmd) {
                    Interrupt::Cancel => return AuthOutcome::Cancelled,
                    Interrupt::Switch(c) => return AuthOutcome::Switch(c),
                    Interrupt::Code(_) | Interrupt::Ignore => continue,
                },
                _ = tokio::time::sleep(interval) => {}
            }
            let poll = conn
                .service_method(CAuthentication_PollAuthSessionStatus_Request {
                    client_id: Some(client_id),
                    request_id: Some(request_id.clone()),
                    ..Default::default()
                })
                .await;
            let r = match poll {
                Ok(r) => r,
                Err(e) if matches!(api_error(&e), Some(EResult::FileNotFound) | Some(EResult::Expired)) => {
                    log!("QR code expired; issuing a new one");
                    continue 'new_code;
                }
                Err(e) if is_transport(&e) => match reconnect(&mut conn, &mut reconnects, &e).await {
                    Ok(()) => continue,
                    Err(msg) => return AuthOutcome::Failed(msg),
                },
                Err(e) => return AuthOutcome::Failed(format!("QR login failed: {e}")),
            };
            reconnects = 0;
            if let Some(id) = r.new_client_id {
                client_id = id;
            }
            if let Some(url) = r.new_challenge_url.clone().filter(|u| !u.is_empty()) {
                state::shared().qr_url = url;
            }
            if r.had_remote_interaction() {
                state::set_state(BridgeState::Authenticating);
            }
            if let Some(token) = r.refresh_token.filter(|t| !t.is_empty()) {
                redact::register(&token);
                return AuthOutcome::LoggedIn(StoredLogin { account_name: r.account_name.unwrap_or_default(), refresh_token: token, guard_data: None });
            }
        }
    }
    AuthOutcome::Failed("QR login timed out".into())
}

fn encrypt_password(modulus_hex: &str, exponent_hex: &str, password: &str) -> Result<String, String> {
    use rsa::{BigUint, Pkcs1v15Encrypt, RsaPublicKey};
    let n = BigUint::parse_bytes(modulus_hex.as_bytes(), 16).ok_or("bad RSA modulus")?;
    let e = BigUint::parse_bytes(exponent_hex.as_bytes(), 16).ok_or("bad RSA exponent")?;
    let key = RsaPublicKey::new(n, e).map_err(|e| e.to_string())?;
    let enc = key.encrypt(&mut rsa::rand_core::OsRng, Pkcs1v15Encrypt, password.as_bytes()).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(enc))
}

fn credentials_error(e: &NetworkError) -> String {
    match api_error(e) {
        Some(EResult::InvalidPassword) => "Wrong account name or password".into(),
        Some(EResult::RateLimitExceeded) | Some(EResult::AccountLoginDeniedThrottle) => "Too many login attempts; wait a while and try again".into(),
        _ => format!("Steam login failed: {e}"),
    }
}

/// `guard_data` is the machine token from an earlier email Steam Guard login of the same account.
pub async fn password_login(rx: &mut UnboundedReceiver<Command>, user: String, pass: String, guard_data: Option<String>) -> AuthOutcome {
    redact::register(&pass);
    state::set_state(BridgeState::Authenticating);
    state::clear_error();
    let mut conn = match connect_unauthenticated().await {
        Ok(c) => c,
        Err(e) => return AuthOutcome::Failed(format!("Could not reach Steam: {e}")),
    };
    let mut reconnects = 0;

    let rsa = match conn.service_method(CAuthentication_GetPasswordRSAPublicKey_Request { account_name: Some(user.clone()), ..Default::default() }).await {
        Ok(r) => r,
        Err(e) => return AuthOutcome::Failed(credentials_error(&e)),
    };
    let encrypted = match encrypt_password(rsa.publickey_mod(), rsa.publickey_exp(), &pass) {
        Ok(p) => p,
        Err(e) => return AuthOutcome::Failed(format!("Could not encrypt the password: {e}")),
    };
    drop(pass);

    let begin = conn
        .service_method(CAuthentication_BeginAuthSessionViaCredentials_Request {
            device_friendly_name: Some(device_name()),
            account_name: Some(user.clone()),
            encrypted_password: Some(encrypted),
            encryption_timestamp: rsa.timestamp,
            remember_login: Some(true),
            platform_type: Some(EnumOrUnknown::new(EAuthTokenPlatformType::k_EAuthTokenPlatformType_SteamClient)),
            persistence: Some(EnumOrUnknown::new(ESessionPersistence::k_ESessionPersistence_Persistent)),
            website_id: Some("Client".into()),
            device_details: MessageField::some(device_details()),
            guard_data: guard_data.clone(),
            ..Default::default()
        })
        .await;
    let begin = match begin {
        Ok(b) => b,
        Err(e) => return AuthOutcome::Failed(credentials_error(&e)),
    };
    let client_id = begin.client_id();
    let steam_id = begin.steamid();
    let request_id = begin.request_id().to_vec();
    let interval = interval_of(begin.interval);

    let has = |t: EAuthSessionGuardType| begin.allowed_confirmations.iter().any(|c| c.confirmation_type() == t);
    // The authenticator app code wins when both are offered; app approval is polled in parallel either way.
    let code_type = if has(EAuthSessionGuardType::k_EAuthSessionGuardType_DeviceCode) {
        Some((EAuthSessionGuardType::k_EAuthSessionGuardType_DeviceCode, "app"))
    } else if has(EAuthSessionGuardType::k_EAuthSessionGuardType_EmailCode) {
        Some((EAuthSessionGuardType::k_EAuthSessionGuardType_EmailCode, "email"))
    } else {
        None
    };
    if let Some((_, kind)) = code_type {
        state::set_state(BridgeState::AwaitingGuardCode);
        state::shared().guard_kind = kind.into();
        log!("Steam Guard code needed ({kind})");
    }

    let deadline = Instant::now() + PASSWORD_LOGIN_DEADLINE;
    while Instant::now() < deadline {
        tokio::select! {
            cmd = rx.recv() => match classify(cmd) {
                Interrupt::Cancel => return AuthOutcome::Cancelled,
                Interrupt::Switch(c) => return AuthOutcome::Switch(c),
                Interrupt::Ignore => continue,
                Interrupt::Code(code) => {
                    redact::register(&code);
                    let Some((ty, _)) = code_type else { continue };
                    state::set_state(BridgeState::Authenticating);
                    let guard_request = || CAuthentication_UpdateAuthSessionWithSteamGuardCode_Request {
                        client_id: Some(client_id),
                        steamid: Some(steam_id),
                        code: Some(code.trim().to_owned()),
                        code_type: Some(EnumOrUnknown::new(ty)),
                        ..Default::default()
                    };
                    let mut sent = conn.service_method(guard_request()).await;
                    // The code usually arrives after a long wait, on a socket Steam has already dropped.
                    if let Err(e) = &sent {
                        if is_transport(e) {
                            if let Err(msg) = reconnect(&mut conn, &mut reconnects, e).await {
                                return AuthOutcome::Failed(msg);
                            }
                            sent = conn.service_method(guard_request()).await;
                        }
                    }
                    match sent {
                        Ok(_) => state::clear_error(),
                        Err(e) if matches!(api_error(&e), Some(EResult::InvalidLoginAuthCode) | Some(EResult::TwoFactorCodeMismatch)) => {
                            state::set_error("That Steam Guard code was not accepted; try again");
                            state::set_state(BridgeState::AwaitingGuardCode);
                            state::shared().guard_kind = code_type.map(|(_, k)| k).unwrap_or_default().into();
                            continue;
                        }
                        Err(e) => return AuthOutcome::Failed(format!("Steam Guard failed: {e}")),
                    }
                }
            },
            _ = tokio::time::sleep(interval) => {}
        }
        let poll = conn
            .service_method(CAuthentication_PollAuthSessionStatus_Request {
                client_id: Some(client_id),
                request_id: Some(request_id.clone()),
                ..Default::default()
            })
            .await;
        match poll {
            Ok(r) => {
                reconnects = 0;
                if let Some(token) = r.refresh_token.clone().filter(|t| !t.is_empty()) {
                    redact::register(&token);
                    return AuthOutcome::LoggedIn(StoredLogin {
                        account_name: r.account_name.clone().filter(|a| !a.is_empty()).unwrap_or(user),
                        refresh_token: token,
                        guard_data: r.new_guard_data.clone().filter(|g| !g.is_empty()).or(guard_data),
                    });
                }
            }
            Err(e) if matches!(api_error(&e), Some(EResult::FileNotFound) | Some(EResult::Expired)) => {
                return AuthOutcome::Failed("The login request expired; please start again".into());
            }
            Err(e) if is_transport(&e) => {
                if let Err(msg) = reconnect(&mut conn, &mut reconnects, &e).await {
                    return AuthOutcome::Failed(msg);
                }
            }
            Err(e) => return AuthOutcome::Failed(format!("Steam login failed: {e}")),
        }
    }
    AuthOutcome::Failed("Login timed out".into())
}
