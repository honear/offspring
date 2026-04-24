use crate::paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

pub fn log_path() -> Option<PathBuf> {
    paths::local_data_dir().ok().map(|d| d.join("debug.log"))
}

pub fn log(msg: &str) {
    let start = START.get_or_init(|| {
        let inst = Instant::now();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(path) = log_path() {
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(
                    f,
                    "\n=== pid {} started at unix {} (decode in PowerShell: [DateTimeOffset]::FromUnixTimeSeconds({}).LocalDateTime) ===",
                    std::process::id(),
                    epoch,
                    epoch
                );
            }
        }
        inst
    });
    let elapsed = start.elapsed().as_secs_f64();
    let Some(path) = log_path() else { return };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{:>5} {:>7.3}s] {}", std::process::id(), elapsed, msg);
    }
}

#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::debug_log::log(&format!($($arg)*))
    };
}
