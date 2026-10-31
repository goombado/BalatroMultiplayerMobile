//! Desktop harness for bmp-steam-bridge. Every call goes through the same `extern "C"` exports the
//! game uses, so the ABI is exercised as well as the login and ticket logic.
//!
//!   bridge-cli login --qr | --password
//!   bridge-cli status
//!   bridge-cli ticket                 # prints a ready auth session ticket as hex
//!   bridge-cli verify                 # checks a fresh ticket with AuthenticateUserTicket (STEAM_WEB_API_KEY)
//!   bridge-cli nakama-login [--host 127.0.0.1] [--port 7350]
//!   bridge-cli logout

use bmp_steam::ffi::*;
use std::ffi::{CStr, CString};
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const STATE_NAMES: [&str; 9] =
    ["UNINITIALISED", "LOGGED_OUT", "AWAITING_QR", "AWAITING_GUARD_CODE", "AUTHENTICATING", "CONNECTING", "READY", "NOT_OWNED", "ERROR"];
const READY: i32 = 6;
const LOGGED_OUT: i32 = 1;
const AWAITING_QR: i32 = 2;
const AWAITING_GUARD: i32 = 3;
const NOT_OWNED: i32 = 7;

fn s(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn state_name(st: i32) -> &'static str {
    STATE_NAMES.get(st as usize).copied().unwrap_or("?")
}

fn data_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("BMP_BRIDGE_DATA_DIR") {
        return d.into();
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| ".".into());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/bmp-steam-bridge-cli")
    } else {
        home.join(".config/bmp-steam-bridge-cli")
    }
}

fn init() {
    bmp_steam::echo_log_to_stderr(std::env::var_os("BMP_BRIDGE_VERBOSE").is_some());
    assert_eq!(bmp_bridge_version(), 1, "unexpected ABI version");
    let dir = data_dir();
    std::fs::create_dir_all(&dir).expect("create data dir");
    let c = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
    unsafe { bmp_bridge_init(c.as_ptr()) };
}

fn prompt(label: &str) -> String {
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn prompt_hidden(label: &str) -> String {
    let _ = std::process::Command::new("stty").arg("-echo").stdin(std::process::Stdio::inherit()).status();
    let v = prompt(label);
    let _ = std::process::Command::new("stty").arg("echo").stdin(std::process::Stdio::inherit()).status();
    eprintln!();
    v
}

fn print_qr(url: &str) {
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => {
            // Two modules per character cell, with a quiet zone, readable by phone cameras.
            let w = code.width();
            let dark = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < w && code[(x as usize, y as usize)] == qrcode::Color::Dark;
            let q = 2isize;
            let mut y = -q;
            while y < w as isize + q {
                let mut line = String::new();
                for x in -q..w as isize + q {
                    line.push(match (dark(x, y), dark(x, y + 1)) {
                        (true, true) => ' ',
                        (true, false) => '▄',
                        (false, true) => '▀',
                        (false, false) => '█',
                    });
                }
                eprintln!("{line}");
                y += 2;
            }
        }
        Err(_) => eprintln!("(could not render QR)"),
    }
    eprintln!("QR URL: {url}");
}

/// Polls the ABI the way the Lua side will, reporting state changes, until `done` says stop.
fn watch(timeout: Duration, mut done: impl FnMut(i32) -> bool) -> i32 {
    let start = Instant::now();
    let mut last_state = -1;
    let mut last_err = String::new();
    let mut last_qr = String::new();
    loop {
        let st = bmp_bridge_state();
        if st != last_state {
            eprintln!("state: {}", state_name(st));
            last_state = st;
        }
        let err = s(bmp_bridge_last_error());
        if err != last_err && !err.is_empty() {
            eprintln!("last error: {err}");
        }
        last_err = err;
        if st == AWAITING_QR {
            let qr = s(bmp_bridge_qr_url());
            if !qr.is_empty() && qr != last_qr {
                print_qr(&qr);
                last_qr = qr;
            }
        }
        if done(st) || start.elapsed() > timeout {
            return st;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn wait_settled(timeout: Duration) -> i32 {
    watch(timeout, |st| st == READY || st == LOGGED_OUT || st == NOT_OWNED)
}

fn login(args: &[String]) {
    init();
    let use_password = args.iter().any(|a| a == "--password");
    // Let a stored login settle first, so we don't race it.
    std::thread::sleep(Duration::from_millis(300));
    if use_password {
        let user = std::env::var("STEAM_ACCOUNT").unwrap_or_else(|_| prompt("Steam account name: "));
        let mut pass = std::env::var("STEAM_PASSWORD").unwrap_or_else(|_| prompt_hidden("Steam password: "));
        let (cu, cp) = (CString::new(user).unwrap(), CString::new(pass.clone()).unwrap());
        unsafe { bmp_bridge_begin_password_login(cu.as_ptr(), cp.as_ptr()) };
        pass.clear();
    } else {
        bmp_bridge_begin_qr_login();
    }
    let st = watch(Duration::from_secs(600), |st| {
        if st == AWAITING_GUARD {
            let kind = s(bmp_bridge_guard_kind());
            let code = prompt(&format!("Steam Guard code ({}), or approve in the Steam app and press Enter: ", if kind.is_empty() { "?" } else { &kind }));
            if !code.is_empty() {
                let c = CString::new(code).unwrap();
                unsafe { bmp_bridge_submit_guard_code(c.as_ptr()) };
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        st == READY || st == NOT_OWNED || (st == LOGGED_OUT && !s(bmp_bridge_last_error()).is_empty())
    });
    report(st);
}

fn report(st: i32) {
    println!("state: {}", state_name(st));
    let id = s(bmp_bridge_steam_id());
    println!("logged in: {}", !id.is_empty());
    println!("persona: {}", s(bmp_bridge_persona_name()));
    let e = s(bmp_bridge_last_error());
    if !e.is_empty() {
        println!("last error: {e}");
    }
    if st != READY {
        std::process::exit(1);
    }
}

fn take_ticket() -> Option<(String, u32)> {
    let user = SteamAPI_SteamUser_v023();
    if user.is_null() {
        return None;
    }
    let mut buf = [0u8; 1024];
    let mut len = 0u32;
    let h = unsafe { SteamAPI_ISteamUser_GetAuthSessionTicket(user, buf.as_mut_ptr().cast(), buf.len() as i32, &mut len, std::ptr::null()) };
    if h == 0 {
        return None;
    }
    Some((buf[..len as usize].iter().map(|b| format!("{b:02x}")).collect(), h))
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Does what the BMP server does with a ticket, while this process still holds the Steam session
/// (Steam drops a session's tickets once it ends, so checking after `ticket` exits can't pass).
fn verify() {
    let Ok(key) = std::env::var("STEAM_WEB_API_KEY") else {
        eprintln!("set STEAM_WEB_API_KEY to your Steam Web API key");
        std::process::exit(2);
    };
    bmp_steam::redact::register(&key);
    init();
    let st = wait_settled(Duration::from_secs(60));
    if st != READY {
        report(st);
    }
    let Some((hex, handle)) = take_ticket() else {
        eprintln!("no ticket ready");
        std::process::exit(1);
    };
    let path = format!("/ISteamUserAuth/AuthenticateUserTicket/v1/?key={key}&appid={}&ticket={hex}", bmp_steam::BALATRO_APP_ID);
    let resp = bmp_steam::net::https_get_blocking("api.steampowered.com", &path);
    SteamAPI_ISteamUser_CancelAuthTicket(std::ptr::null_mut(), handle);
    match resp {
        Ok(body) => {
            let body = String::from_utf8_lossy(&body);
            // Only the verdict fields; the full response also carries the SteamIDs.
            let pick = |k: &str| body.split(&format!("\"{k}\":")).nth(1).map(|v| v.split([',', '}']).next().unwrap_or("").to_owned());
            println!("result: {}", pick("result").unwrap_or_else(|| "(none)".into()));
            for k in ["vacbanned", "publisherbanned", "errorcode", "errordesc"] {
                if let Some(v) = pick(k) {
                    println!("{k}: {v}");
                }
            }
        }
        Err(e) => {
            eprintln!("AuthenticateUserTicket failed: {}", bmp_steam::redact::redact(&e));
            std::process::exit(1);
        }
    }
}

fn nakama_login(args: &[String]) {
    let host = arg_value(args, "--host").unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = arg_value(args, "--port").and_then(|p| p.parse().ok()).unwrap_or(7350);
    if host.to_ascii_lowercase().contains("balatromp.com") {
        eprintln!("Refusing to talk to an official BMP server from the test harness.");
        std::process::exit(2);
    }
    init();
    let st = wait_settled(Duration::from_secs(60));
    if st != READY {
        report(st);
    }
    let Some((hex, handle)) = take_ticket() else {
        eprintln!("no ticket ready");
        std::process::exit(1);
    };
    let body = format!(r#"{{"id":"steam-ticket","vars":{{"steam_ticket":"{hex}"}}}}"#);
    // base64("defaultkey:")
    let req = format!(
        "POST /v2/account/authenticate/custom?create=true HTTP/1.0\r\nHost: {host}:{port}\r\nAuthorization: Basic ZGVmYXVsdGtleTo=\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut resp = String::new();
    let sent = TcpStream::connect((host.as_str(), port)).and_then(|mut t| {
        t.set_read_timeout(Some(Duration::from_secs(20)))?;
        t.write_all(req.as_bytes())?;
        t.read_to_string(&mut resp)
    });
    SteamAPI_ISteamUser_CancelAuthTicket(std::ptr::null_mut(), handle);
    std::thread::sleep(Duration::from_millis(500));
    if let Err(e) = sent {
        eprintln!("request failed: {e}");
        std::process::exit(1);
    }
    let status = resp.split_whitespace().nth(1).unwrap_or("?").to_owned();
    let got_token = resp.contains("\"token\"");
    println!("HTTP {status}; session token returned: {got_token}");
    if !got_token {
        if let Some(body) = resp.split("\r\n\r\n").nth(1) {
            println!("server said: {body}");
        }
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("login") => login(&args[1..]),
        Some("status") => {
            init();
            report(wait_settled(Duration::from_secs(60)));
        }
        Some("ticket") => {
            init();
            let st = wait_settled(Duration::from_secs(60));
            if st != READY {
                report(st);
            }
            match take_ticket() {
                Some((hex, handle)) => {
                    println!("{hex}");
                    eprintln!("handle {handle}, {} bytes. Not cancelled; it ends when this process exits.", hex.len() / 2);
                }
                None => {
                    eprintln!("no ticket ready");
                    std::process::exit(1);
                }
            }
        }
        Some("verify") => verify(),
        Some("nakama-login") => nakama_login(&args[1..]),
        Some("logout") => {
            init();
            std::thread::sleep(Duration::from_millis(300));
            bmp_bridge_logout();
            watch(Duration::from_secs(15), |st| st == LOGGED_OUT);
            println!("logged out; stored token deleted");
        }
        _ => {
            eprintln!("usage: bridge-cli login --qr|--password | status | ticket | verify | nakama-login [--host H] [--port P] | logout");
            eprintln!("env: BMP_BRIDGE_DATA_DIR (token/log location), BMP_BRIDGE_VERBOSE=1 (log to stderr), STEAM_WEB_API_KEY (verify only)");
            std::process::exit(2);
        }
    }
}
