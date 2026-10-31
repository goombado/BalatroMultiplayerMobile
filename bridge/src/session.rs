//! One logged-in CM session: logon with the refresh token, then keep a small pool of activated
//! Balatro auth session tickets until the socket drops, Steam logs us off, or the user logs out.
//!
//! The ticket flow is ported from node-steam-user `components/appauth.js` (MIT):
//! - Collect GC tokens (ClientGameConnectTokens).
//! - Get the app ownership ticket.
//! - Build GC token + session header + ownership ticket.
//! - Register the auth portion with Steam through ClientAuthList.
//!
//! A ticket is only handed out after the AuthList ack, so `AuthenticateUserTicket` accepts it right
//! away (spike 0.1).

use crate::log::log;
use crate::state::{self, BridgeState, Command, ReadyTicket};
use crate::store::{decode_claims, Store, StoredLogin};
use crate::ticket::{self, SessionHeader};
use crate::{net, redact, BALATRO_APP_ID};
use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use steam_vent::connection::UnAuthenticatedConnection;
use steam_vent::{Connection, ConnectionError, ConnectionTrait, EResult, LoginError, NetworkError};
use steam_vent_proto_common::protobuf::EnumOrUnknown;
use steam_vent_proto_steam::steammessages_auth_steamclient::{CAuthentication_AccessToken_GenerateForApp_Request, ETokenRenewalType};
use steam_vent_proto_steam::steammessages_base::CMsgAuthTicket;
use steam_vent_proto_steam::steammessages_clientserver::{
    cmsg_client_games_played, CMsgClientAuthList, CMsgClientAuthListAck, CMsgClientGameConnectTokens, CMsgClientGamesPlayed, CMsgClientGetAppOwnershipTicket,
    CMsgClientGetAppOwnershipTicketResponse,
};
use steam_vent_proto_steam::steammessages_clientserver_login::{CMsgClientAccountInfo, CMsgClientLoggedOff};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_stream::StreamExt;

/// Tickets kept activated and ready. Two, so a second request right after the first (MPAPI's retry on
/// Steam's "101 invalid ticket") never finds the pool empty while the next one is minted.
const POOL_TARGET: usize = 2;
/// Renew the refresh token once it has less than this left (SteamClient tokens last about 200 days).
const RENEW_WHEN_LEFT: Duration = Duration::from_secs(30 * 24 * 3600);
const OWNERSHIP_RECHECK: Duration = Duration::from_secs(30 * 60);

pub enum Ended {
    LoggedOut,
    /// Steam no longer accepts the stored token: log in again.
    TokenRejected(String),
    NotOwned,
    /// The socket dropped or Steam ended the session: reconnect with backoff.
    Disconnected(String),
    /// LoggedInElsewhere and similar: wait longer before reconnecting.
    Kicked(String),
    /// A login command arrived while logged in (e.g. a different account).
    Switch(Command),
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn token_rejected(e: &ConnectionError) -> bool {
    matches!(
        e,
        ConnectionError::LoginError(LoginError::InvalidCredentials)
            | ConnectionError::AccessToken(_)
            | ConnectionError::LoginError(LoginError::Unknown(EResult::AccessDenied | EResult::Expired | EResult::Revoked))
    )
}

struct ActiveTicket {
    handle: u32,
    crc: u32,
    auth: Vec<u8>,
}

struct Session<'a> {
    conn: Connection,
    store: &'a Store,
    login: StoredLogin,
    logon_at: Instant,
    connection_count: u32,
    public_ip: u32,
    gc_tokens: VecDeque<Vec<u8>>,
    ownership: Option<Vec<u8>>,
    active: Vec<ActiveTicket>,
    next_handle: u32,
    seq_me: u32,
    seq_them: u32,
    h_steam_pipe: u32,
}

pub async fn run(store: &Store, login: StoredLogin, rx: &mut UnboundedReceiver<Command>, next_handle: &mut u32) -> Ended {
    state::set_state(BridgeState::Connecting);
    // Zero until logon succeeds; the worker reads it to tell a dropped session from a failed connect.
    state::shared().steam_id = 0;
    redact::register(&login.refresh_token);

    let servers = match net::cm_servers().await {
        Ok(s) => s,
        Err(e) => return Ended::Disconnected(e),
    };
    let mut attempt = Err(String::from("no CM server"));
    for endpoint in servers.iter().take(3) {
        attempt = match net::connect_cm(endpoint).await {
            Ok((sink, stream, closed)) => match UnAuthenticatedConnection::from_sender_receiver(sink, stream).await {
                Ok(c) => Ok((c, closed)),
                Err(e) => Err(format!("hello to {endpoint}: {e}")),
            },
            Err(e) => Err(e),
        };
        if attempt.is_ok() {
            break;
        }
    }
    let (unauth, mut closed) = match attempt {
        Ok(x) => x,
        Err(e) => return Ended::Disconnected(e),
    };

    // CM logon takes the refresh token in the access_token field; steam-vent's `access` does that.
    let conn = match unauth.access(&login.account_name, &login.refresh_token).await {
        Ok(c) => c,
        Err(e) if token_rejected(&e) => return Ended::TokenRejected(format!("Steam rejected the saved login ({e}); please log in again")),
        Err(e) => return Ended::Disconnected(format!("logon failed: {e}")),
    };

    // Subscribe before draining the backlog so nothing that arrives in between is lost.
    let mut gc_stream = conn.on::<CMsgClientGameConnectTokens>();
    let mut account_stream = conn.on::<CMsgClientAccountInfo>();
    let mut logged_off = conn.on::<CMsgClientLoggedOff>();

    let public_ip = match conn.public_ip() {
        Some(IpAddr::V4(v4)) => u32::from(v4),
        _ => 0,
    };
    let steam_id = u64::from(conn.steam_id());
    log!("logged on (public IPv4 known: {})", public_ip != 0);
    {
        let mut s = state::shared();
        s.steam_id = steam_id;
    }

    let mut session = Session {
        conn,
        store,
        login,
        logon_at: Instant::now(),
        connection_count: 0,
        public_ip,
        gc_tokens: VecDeque::new(),
        ownership: None,
        active: Vec::new(),
        next_handle: *next_handle,
        seq_me: 0,
        seq_them: 0,
        h_steam_pipe: rand::random_range(1..1_000_000),
    };

    for raw in session.conn.take_unprocessed() {
        if let Ok(m) = raw.clone().into_message::<CMsgClientGameConnectTokens>() {
            session.add_gc_tokens(m);
        } else if let Ok(m) = raw.into_message::<CMsgClientAccountInfo>() {
            set_persona(&m);
        }
    }

    let ended = session.serve(rx, &mut gc_stream, &mut account_stream, &mut logged_off, &mut closed).await;
    *next_handle = session.next_handle;
    {
        let mut s = state::shared();
        s.ticket_pool.clear();
        if !matches!(ended, Ended::Disconnected(_) | Ended::Kicked(_)) {
            s.steam_id = 0;
            s.persona_name.clear();
        }
    }
    ended
}

fn set_persona(info: &CMsgClientAccountInfo) {
    if let Some(name) = info.persona_name.as_ref().filter(|n| !n.is_empty()) {
        state::shared().persona_name = name.clone();
    }
}

impl Session<'_> {
    fn add_gc_tokens(&mut self, m: CMsgClientGameConnectTokens) {
        self.gc_tokens.extend(m.tokens);
        let keep = m.max_tokens_to_keep.unwrap_or(10).max(1) as usize;
        while self.gc_tokens.len() > keep {
            self.gc_tokens.pop_front();
        }
        log!("GC tokens available: {}", self.gc_tokens.len());
    }

    async fn serve(
        &mut self,
        rx: &mut UnboundedReceiver<Command>,
        gc_stream: &mut (impl tokio_stream::Stream<Item = Result<CMsgClientGameConnectTokens, NetworkError>> + Unpin),
        account_stream: &mut (impl tokio_stream::Stream<Item = Result<CMsgClientAccountInfo, NetworkError>> + Unpin),
        logged_off: &mut (impl tokio_stream::Stream<Item = Result<CMsgClientLoggedOff, NetworkError>> + Unpin),
        closed: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Ended {
        self.maybe_renew_token().await;
        if std::env::var_os("BMP_BRIDGE_GAMES_PLAYED").is_some() {
            // Spike 0.1: tickets validate without it, so it is opt-in (it also shows "In-Game" on the profile).
            let _ = self
                .conn
                .send(CMsgClientGamesPlayed {
                    games_played: vec![cmsg_client_games_played::GamePlayed { game_id: Some(BALATRO_APP_ID as u64), ..Default::default() }],
                    ..Default::default()
                })
                .await;
        }

        match self.ensure_ownership().await {
            Ok(()) => {}
            Err(OwnershipError::NotOwned) => return Ended::NotOwned,
            Err(OwnershipError::Other(e)) => return Ended::Disconnected(e),
        }

        let mut recheck = tokio::time::interval(OWNERSHIP_RECHECK);
        recheck.tick().await;
        loop {
            if let Err(e) = self.fill_pool().await {
                return Ended::Disconnected(e);
            }
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    None | Some(Command::Logout) => return Ended::LoggedOut,
                    Some(c @ Command::BeginQr) | Some(c @ Command::BeginPassword { .. }) => return Ended::Switch(c),
                    Some(Command::CancelTicket(h)) => {
                        if let Err(e) = self.cancel(h).await {
                            return Ended::Disconnected(e);
                        }
                    }
                    Some(Command::Refill) | Some(Command::GuardCode(_)) | Some(Command::CancelLogin) => {}
                },
                Some(Ok(m)) = gc_stream.next() => self.add_gc_tokens(m),
                Some(Ok(m)) = account_stream.next() => set_persona(&m),
                Some(Ok(m)) = logged_off.next() => {
                    let r = m.eresult.unwrap_or(0);
                    log!("Steam logged us off (eresult {r})");
                    return match EResult::from_result(r) {
                        Err(EResult::LoggedInElsewhere) | Err(EResult::AlreadyLoggedInElsewhere) =>
                            Ended::Kicked("This Steam account logged in somewhere else".into()),
                        Err(EResult::AccessDenied) | Err(EResult::Revoked) | Err(EResult::Expired) =>
                            Ended::TokenRejected("Steam ended the saved login; please log in again".into()),
                        _ => Ended::Disconnected(format!("logged off by Steam (eresult {r})")),
                    };
                },
                _ = closed.changed() => return Ended::Disconnected("connection to Steam closed".into()),
                _ = recheck.tick() => {
                    if let Err(OwnershipError::Other(e)) = self.ensure_ownership().await {
                        return Ended::Disconnected(e);
                    }
                }
            }
        }
    }

    async fn maybe_renew_token(&mut self) {
        let Some(claims) = decode_claims(&self.login.refresh_token) else { return };
        if claims.exp > now_unix() + RENEW_WHEN_LEFT.as_secs() {
            return;
        }
        let res = self
            .conn
            .service_method(CAuthentication_AccessToken_GenerateForApp_Request {
                refresh_token: Some(self.login.refresh_token.clone()),
                steamid: claims.sub.parse().ok(),
                renewal_type: Some(EnumOrUnknown::new(ETokenRenewalType::k_ETokenRenewalType_Allow)),
                ..Default::default()
            })
            .await;
        match res {
            Ok(r) => {
                if let Some(t) = r.refresh_token.filter(|t| !t.is_empty()) {
                    redact::register(&t);
                    self.login.refresh_token = t;
                    match self.store.save(&self.login) {
                        Ok(()) => log!("refresh token renewed"),
                        Err(e) => log!("renewed token could not be saved: {e}"),
                    }
                }
            }
            Err(e) => log!("token renewal failed: {e}"),
        }
    }

    async fn ensure_ownership(&mut self) -> Result<(), OwnershipError> {
        if let Some(info) = self.ownership.as_deref().and_then(ticket::parse_ownership) {
            if ticket::ownership_reusable(&info, now_unix(), self.public_ip) {
                return Ok(());
            }
        }
        let res: Result<CMsgClientGetAppOwnershipTicketResponse, _> =
            self.conn.job(CMsgClientGetAppOwnershipTicket { app_id: Some(BALATRO_APP_ID), ..Default::default() }).await;
        let res = match res {
            Ok(r) => r,
            Err(NetworkError::ApiError(EResult::AccessDenied)) => return Err(OwnershipError::NotOwned),
            Err(e) => return Err(OwnershipError::Other(format!("ownership ticket request failed: {e}"))),
        };
        match EResult::from_result(res.eresult.unwrap_or(2) as i32) {
            Ok(()) => {}
            Err(EResult::AccessDenied) => return Err(OwnershipError::NotOwned),
            Err(r) => return Err(OwnershipError::Other(format!("ownership ticket refused: {r:?}"))),
        }
        let t = res.ticket.unwrap_or_default();
        // An account without the game gets an empty or stub ticket rather than an error.
        if t.len() <= 10 || res.app_id != Some(BALATRO_APP_ID) {
            return Err(OwnershipError::NotOwned);
        }
        if let Some(info) = ticket::parse_ownership(&t) {
            log!("ownership ticket ok (expires {})", info.expires);
        }
        self.ownership = Some(t);
        Ok(())
    }

    async fn fill_pool(&mut self) -> Result<(), String> {
        loop {
            let have = state::shared().ticket_pool.len();
            if have >= POOL_TARGET || self.gc_tokens.is_empty() {
                return Ok(());
            }
            let t = self.mint().await?;
            {
                let mut s = state::shared();
                s.ticket_pool.push_back(t);
            }
            state::set_state(BridgeState::Ready);
        }
    }

    async fn mint(&mut self) -> Result<ReadyTicket, String> {
        let ownership = self.ownership.clone().ok_or("no ownership ticket")?;
        let gc = self.gc_tokens.pop_front().ok_or("no GC token")?;
        self.connection_count += 1;
        let header = SessionHeader {
            public_ip: self.public_ip,
            ms_since_logon: self.logon_at.elapsed().as_millis().min(u32::MAX as u128) as u32,
            connection_count: self.connection_count,
        };
        let bytes = ticket::build_session_ticket(&gc, &header, &ownership);
        let auth = ticket::auth_portion(&bytes).ok_or("built a malformed ticket")?.to_vec();
        let crc = crc32fast::hash(&auth);
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        let handle = self.next_handle;
        self.active.push(ActiveTicket { handle, crc, auth });
        self.send_auth_list().await?;
        log!("ticket {handle} activated ({} bytes)", bytes.len());
        Ok(ReadyTicket { handle, bytes })
    }

    async fn cancel(&mut self, handle: u32) -> Result<(), String> {
        let before = self.active.len();
        self.active.retain(|t| t.handle != handle);
        state::shared().ticket_pool.retain(|t| t.handle != handle);
        if self.active.len() != before {
            log!("ticket {handle} cancelled");
            self.send_auth_list().await?;
        }
        Ok(())
    }

    async fn send_auth_list(&mut self) -> Result<(), String> {
        self.seq_me += 1;
        let msg = CMsgClientAuthList {
            tokens_left: Some(self.gc_tokens.len() as u32),
            last_request_seq: Some(self.seq_me - 1),
            last_request_seq_from_server: Some(self.seq_them),
            tickets: self
                .active
                .iter()
                .map(|t| CMsgAuthTicket {
                    estate: Some(0),
                    steamid: Some(0),
                    gameid: Some(BALATRO_APP_ID as u64),
                    h_steam_pipe: Some(self.h_steam_pipe),
                    ticket_crc: Some(t.crc),
                    ticket: Some(t.auth.clone()),
                    ..Default::default()
                })
                .collect(),
            app_ids: vec![BALATRO_APP_ID],
            message_sequence: Some(self.seq_me),
            ..Default::default()
        };
        let ack: CMsgClientAuthListAck = self.conn.job(msg).await.map_err(|e| format!("auth list not acknowledged: {e}"))?;
        self.seq_them = ack.message_sequence();
        Ok(())
    }
}

enum OwnershipError {
    NotOwned,
    Other(String),
}
