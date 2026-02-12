use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

pub(crate) const DEBUG_LOG_ENV_VAR: &str = "HYPRFINITY_DEBUG_LOG";
pub(crate) const DEFAULT_DEBUG_LOG_PATH: &str = "/var/log/hyprfinity-debug.log";
pub(crate) const FALLBACK_DEBUG_LOG_PATH: &str = "/tmp/hyprfinity-debug.log";

static DEBUG_LOGGER: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

pub(crate) fn init_debug_logging(
    enabled: bool,
    path_override: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(());
    }
    let chosen_path = if let Some(p) = path_override.as_ref() {
        PathBuf::from(p)
    } else if let Ok(p) = std::env::var(DEBUG_LOG_ENV_VAR) {
        PathBuf::from(p)
    } else {
        PathBuf::from(DEFAULT_DEBUG_LOG_PATH)
    };

    let open_file = |path: &PathBuf| -> Result<std::fs::File, Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Ok(std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?)
    };

    let (path, file) = match open_file(&chosen_path) {
        Ok(f) => (chosen_path, f),
        Err(e) => {
            let fallback = PathBuf::from(FALLBACK_DEBUG_LOG_PATH);
            eprintln!(
                "Hyprfinity: Failed to open debug log at {} ({}), falling back to {}",
                chosen_path.display(),
                e,
                fallback.display()
            );
            let f = open_file(&fallback)?;
            (fallback, f)
        }
    };

    let _ = DEBUG_LOGGER.set(Mutex::new(file));
    println!("Hyprfinity: Debug log enabled at {}", path.display());
    debug_log_line("debug logging initialized");
    Ok(())
}

pub(crate) fn debug_log_line(message: &str) {
    let Some(lock) = DEBUG_LOGGER.get() else {
        return;
    };
    let ts_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = lock.lock() {
        let _ = writeln!(file, "[{}] {}", ts_ms, message);
    }
}
