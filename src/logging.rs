use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Initialize logging to ~/Library/Logs/ClaudeSleepPreventer/ccsp.log
pub fn init() {
    let log_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Logs/ClaudeSleepPreventer");

    if fs::create_dir_all(&log_dir).is_ok() {
        let log_path = log_dir.join("ccsp.log");
        *LOG_FILE.lock().unwrap() = Some(log_path.clone());

        // Write startup message
        log_internal(&format!("=== CCSP {} started ===", env!("CARGO_PKG_VERSION")));
        log_internal(&format!("Executable: {:?}", std::env::current_exe().ok()));
    }
}

/// Log a message with timestamp
pub fn log(message: &str) {
    log_internal(message);
    eprintln!("{}", message);
}

fn log_internal(message: &str) {
    let log_path = LOG_FILE.lock().unwrap();
    if let Some(path) = log_path.as_ref() {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(file, "[{}] {}", timestamp, message);
        }
    }
}

/// Get the log file path for users to find
pub fn log_path() -> Option<PathBuf> {
    LOG_FILE.lock().unwrap().clone()
}
