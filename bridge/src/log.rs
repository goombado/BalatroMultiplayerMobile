//! A small append-only log in the data dir (`bmp_steam_bridge.log`), for device test reports.
//! Every line goes through `redact`. The file is cut back to empty once it passes 256 KiB.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 256 * 1024;
pub const LOG_FILE_NAME: &str = "bmp_steam_bridge.log";

static SINK: Mutex<Option<File>> = Mutex::new(None);
static ECHO_STDERR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn init(data_dir: &Path) {
    let path = data_dir.join(LOG_FILE_NAME);
    let too_big = std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false);
    let file = OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(&path).ok();
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = file;
}

/// The desktop CLI mirrors the log to stderr.
pub fn echo_to_stderr(on: bool) {
    ECHO_STDERR.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn write(msg: &str) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = format!("[{secs}] {}\n", crate::redact::redact(msg));
    if ECHO_STDERR.load(std::sync::atomic::Ordering::Relaxed) {
        eprint!("{line}");
    }
    if let Some(f) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = f.write_all(line.as_bytes());
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
pub(crate) use log;
