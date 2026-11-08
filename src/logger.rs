use chrono::Local;
use directories::ProjectDirs;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Determine the log file path, preferring `~/.local/state/stoat-tui/debug.log` on Linux,
/// falling back to the system temporary directory (`/tmp/stoat-tui.log`).
pub fn resolve_log_path() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("chat", "stoat", "stoat-tui") {
        let state_dir = proj_dirs.state_dir().unwrap_or_else(|| proj_dirs.data_local_dir());
        if fs::create_dir_all(state_dir).is_ok() {
            return state_dir.join("debug.log");
        }
    }

    let fallback_dir = std::env::temp_dir();
    fallback_dir.join("stoat-tui.log")
}

/// Initialize the logger. Creates parent directories and appends an initialization entry.
pub fn init() -> PathBuf {
    let path = resolve_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    {
        let mut guard = LOG_PATH.lock().unwrap();
        *guard = Some(path.clone());
    }

    log("INFO", &format!("=== Stoat TUI Logger Initialized (v{}, PID: {}) ===", env!("CARGO_PKG_VERSION"), std::process::id()));
    log("INFO", &format!("Log destination: {}", path.display()));
    path
}

/// Get the active log file path
pub fn current_log_path() -> PathBuf {
    if let Ok(guard) = LOG_PATH.lock() {
        if let Some(ref p) = *guard {
            return p.clone();
        }
    }
    resolve_log_path()
}

/// Append a log message with level and timestamp
pub fn log(level: &str, message: &str) {
    let path = current_log_path();
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{}] [{}] {}\n", timestamp, level, message);

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logger::log("INFO", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logger::log("WARN", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logger::log("ERROR", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logger::log("DEBUG", &format!($($arg)*))
    };
}
