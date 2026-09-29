//! The error-only log: scheduler failures, task launch failures, and failed
//! state saves.
//!
//! Task logs hold full transcripts; this file holds just the errors, so a
//! problem is easy to find without wading through normal output. It is written
//! at error level only, and never pruned by the task-log retention window.

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::state::{now, AppState};

/// File name (and stem) of the error log inside the app's `logs/` directory.
pub const ERROR_LOG: &str = "errors.log";

/// Format unix seconds as an RFC 3339-ish UTC timestamp, without a date crate.
fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

pub fn path(logs_dir: &Path) -> PathBuf {
    logs_dir.join(ERROR_LOG)
}

/// Append one error line. Best-effort: a logging failure must never break the
/// caller (which is often already handling an error).
pub fn append(logs_dir: &Path, context: &str, message: &str) {
    let _ = std::fs::create_dir_all(logs_dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(logs_dir))
    {
        let _ = writeln!(f, "[{}] [{}] {}", iso(now()), context, message);
    }
}

/// Read the whole error log, or an empty string when there is none.
pub fn read(logs_dir: &Path) -> String {
    std::fs::read_to_string(path(logs_dir)).unwrap_or_default()
}

/// Truncate the error log. Missing is fine.
pub fn clear(logs_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path(logs_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Append an error through the app handle.
pub fn record(app: &AppHandle, context: &str, message: &str) {
    let st = app.state::<AppState>();
    append(&st.logs_dir(), context, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_unix_seconds_as_utc() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_000_000_000), "2001-09-09T01:46:40Z");
        // The adaptive-learn provider failure, as a real-world cross-check.
        assert_eq!(iso(1_790_702_658), "2026-09-29T17:24:18Z");
    }

    #[test]
    fn append_accumulates_error_lines_with_context() {
        let dir = std::env::temp_dir().join(format!("solayge-errlog-{}", uuid::Uuid::new_v4()));
        append(&dir, "scheduler", "tick failed");
        append(&dir, "task", "failed to launch opencode");

        let text = read(&dir);
        assert_eq!(text.lines().count(), 2, "only the two errors are logged");
        assert!(text.contains("[scheduler] tick failed"));
        assert!(text.contains("[task] failed to launch opencode"));
        assert!(text.contains("T"), "timestamp is formatted");

        clear(&dir).unwrap();
        assert_eq!(read(&dir), "");
        // Clearing an already-cleared (missing) log is fine.
        clear(&dir).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
