use crate::paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
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

/// Redact a string that may be a filesystem path so the debug log
/// doesn't capture the user's directory tree. Filenames are kept
/// (preserves debug usefulness — "the failing file was foo.mp4" is
/// far more actionable than "[redacted-path]") but anything before the
/// last separator is collapsed to `…/`.
///
/// CLI flags (anything starting with `-`) and bare strings without a
/// separator pass through unchanged. The intent is to redact the
/// argv elements that are file paths — the elements that aren't (verb
/// names, --id values) keep their full form.
pub fn redact_path(s: &str) -> String {
    if s.starts_with('-') {
        return s.to_string();
    }
    if !s.contains('\\') && !s.contains('/') {
        return s.to_string();
    }
    Path::new(s)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| format!("…/{n}"))
        .unwrap_or_else(|| "[redacted-path]".to_string())
}

/// Redact every element in an argv-shaped slice. Used by the IPC
/// listener and the primary's startup logging so file paths from
/// users' selections never end up in `debug.log` verbatim.
pub fn redact_argv<S: AsRef<str>>(argv: &[S]) -> Vec<String> {
    argv.iter().map(|s| redact_path(s.as_ref())).collect()
}

#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::debug_log::log(&format!($($arg)*))
    };
}
