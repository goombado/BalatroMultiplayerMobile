//! The single background thread. It owns a current-thread tokio runtime (no thread pool) and runs
//! the login/session state machine. It talks to the C ABI only through `state`.

use crate::auth::{self, AuthOutcome};
use crate::log::log;
use crate::session::{self, Ended};
use crate::state::{self, BridgeState, Command};
use crate::store::{Store, StoredLogin};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(120);
const KICKED_WAIT: Duration = Duration::from_secs(5 * 60);

pub fn start(data_dir: PathBuf) -> Result<(), String> {
    let (tx, rx) = unbounded_channel();
    if state::COMMANDS.set(tx).is_err() {
        return Ok(()); // already running
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    std::thread::Builder::new().name("bmp-steam-bridge".into()).spawn(move || runtime.block_on(run(Store::new(&data_dir), rx))).map_err(|e| e.to_string())?;
    Ok(())
}

/// Waits out a reconnect delay, but a command (logout, new login) cuts it short.
async fn wait_or_command(rx: &mut UnboundedReceiver<Command>, d: Duration) -> Option<Command> {
    tokio::select! {
        cmd = rx.recv() => cmd,
        _ = tokio::time::sleep(d) => None,
    }
}

async fn run(store: Store, mut rx: UnboundedReceiver<Command>) {
    let mut login: Option<StoredLogin> = store.load();
    let mut pending: Option<Command> = None;
    let mut backoff = BACKOFF_START;
    let mut next_handle: u32 = 0;
    log!("worker started (stored login: {})", login.is_some());

    loop {
        // A login command always wins over the stored login.
        if let Some(cmd) = pending.take() {
            match cmd {
                Command::BeginQr | Command::BeginPassword { .. } => {
                    let guard_data = login.as_ref().and_then(|l| l.guard_data.clone());
                    let outcome = match cmd {
                        Command::BeginQr => auth::qr_login(&mut rx).await,
                        Command::BeginPassword { user, pass } => {
                            let guard = guard_data.filter(|_| login.as_ref().is_some_and(|l| l.account_name.eq_ignore_ascii_case(&user)));
                            auth::password_login(&mut rx, user, pass, guard).await
                        }
                        _ => unreachable!(),
                    };
                    match outcome {
                        AuthOutcome::LoggedIn(new_login) => {
                            if let Err(e) = store.save(&new_login) {
                                log!("could not save the login: {e}");
                            }
                            login = Some(new_login);
                            backoff = BACKOFF_START;
                            state::clear_error();
                        }
                        AuthOutcome::Cancelled => {}
                        AuthOutcome::Switch(c) => pending = Some(c),
                        AuthOutcome::Failed(e) => state::set_error(&e),
                    }
                }
                Command::Logout => {
                    store.delete();
                    login = None;
                    state::clear_error();
                }
                _ => {}
            }
            continue;
        }

        let Some(current) = login.clone() else {
            state::set_state(BridgeState::LoggedOut);
            pending = rx.recv().await;
            if pending.is_none() {
                return;
            }
            continue;
        };

        match session::run(&store, current, &mut rx, &mut next_handle).await {
            Ended::LoggedOut => {
                log!("logged out");
                store.delete();
                login = None;
                state::clear_error();
            }
            Ended::Switch(c) => pending = Some(c),
            Ended::TokenRejected(e) => {
                store.delete();
                login = None;
                state::set_error(&e);
            }
            Ended::NotOwned => {
                state::set_error("This Steam account does not own Balatro");
                state::set_state(BridgeState::NotOwned);
                // Stay here until the user logs out or logs in with another account.
                pending = rx.recv().await;
                if pending.is_none() {
                    return;
                }
                if matches!(pending, Some(Command::Logout)) {
                    store.delete();
                    login = None;
                    pending = None;
                }
            }
            Ended::Disconnected(e) => {
                // A session that got as far as logon was healthy; start the backoff over.
                if state::shared().steam_id != 0 {
                    backoff = BACKOFF_START;
                }
                state::set_error(&e);
                state::set_state(BridgeState::Connecting);
                log!("reconnecting in {}s", backoff.as_secs());
                pending = wait_or_command(&mut rx, backoff).await.filter(|c| !matches!(c, Command::Refill | Command::CancelTicket(_)));
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
            Ended::Kicked(e) => {
                state::set_error(&e);
                state::set_state(BridgeState::Error);
                pending = wait_or_command(&mut rx, KICKED_WAIT).await.filter(|c| !matches!(c, Command::Refill | Command::CancelTicket(_)));
            }
        }
        if matches!(pending, Some(Command::Logout)) {
            store.delete();
            login = None;
            pending = None;
            state::clear_error();
        }
    }
}
