//! State shared between the worker thread and the C ABI.
//!
//! The ABI must answer in microseconds and never wait on the network, so it only reads this struct
//! under a short lock and posts `Command`s to the worker. Pre-activated tickets sit in `ticket_pool`,
//! so `GetAuthSessionTicket` can return synchronously, the way Steamworks does.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tokio::sync::mpsc::UnboundedSender;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    Uninitialised = 0,
    LoggedOut = 1,
    AwaitingQr = 2,
    AwaitingGuardCode = 3,
    Authenticating = 4,
    Connecting = 5,
    Ready = 6,
    NotOwned = 7,
    Error = 8,
}

#[derive(Debug, Clone)]
pub struct ReadyTicket {
    pub handle: u32,
    pub bytes: Vec<u8>,
}

pub struct Shared {
    pub state: BridgeState,
    pub last_error: String,
    pub qr_url: String,
    /// What kind of Steam Guard code AWAITING_GUARD_CODE wants: "email", "app" or "".
    pub guard_kind: String,
    pub steam_id: u64,
    pub persona_name: String,
    pub ticket_pool: VecDeque<ReadyTicket>,
}

impl Shared {
    const fn new() -> Self {
        Shared {
            state: BridgeState::Uninitialised,
            last_error: String::new(),
            qr_url: String::new(),
            guard_kind: String::new(),
            steam_id: 0,
            persona_name: String::new(),
            ticket_pool: VecDeque::new(),
        }
    }
}

static SHARED: Mutex<Shared> = Mutex::new(Shared::new());

pub fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_state(state: BridgeState) {
    let mut s = shared();
    if s.state != state {
        crate::log::log!("state {:?} -> {:?}", s.state, state);
        s.state = state;
    }
    if state != BridgeState::AwaitingQr {
        s.qr_url.clear();
    }
    if state != BridgeState::AwaitingGuardCode {
        s.guard_kind.clear();
    }
    if state != BridgeState::Ready {
        s.ticket_pool.clear();
    }
}

pub fn set_error(msg: &str) {
    let msg = crate::redact::redact(msg);
    crate::log::log!("error: {msg}");
    shared().last_error = msg;
}

pub fn clear_error() {
    shared().last_error.clear();
}

pub enum Command {
    BeginQr,
    BeginPassword {
        user: String,
        pass: String,
    },
    GuardCode(String),
    CancelLogin,
    Logout,
    /// A ticket left the pool; mint another.
    Refill,
    CancelTicket(u32),
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::BeginQr => write!(f, "BeginQr"),
            Command::BeginPassword { .. } => write!(f, "BeginPassword"),
            Command::GuardCode(_) => write!(f, "GuardCode"),
            Command::CancelLogin => write!(f, "CancelLogin"),
            Command::Logout => write!(f, "Logout"),
            Command::Refill => write!(f, "Refill"),
            Command::CancelTicket(h) => write!(f, "CancelTicket({h})"),
        }
    }
}

pub static COMMANDS: OnceLock<UnboundedSender<Command>> = OnceLock::new();

pub fn send(cmd: Command) {
    if let Some(tx) = COMMANDS.get() {
        let _ = tx.send(cmd);
    }
}
