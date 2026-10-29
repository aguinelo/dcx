use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SIZE: u64 = 2 * 1024 * 1024; // 2 MB

pub struct DaemonLog {
    path: PathBuf,
}

impl DaemonLog {
    /// Opens (or creates) the daemon log at `{home}/.dcx/daemon.log`.
    /// Rotates to `daemon.log.1` if the file exceeds `MAX_SIZE`.
    pub fn open(home_dir: &Path) -> Result<Self, String> {
        let dir = home_dir.join(".dcx");
        fs::create_dir_all(&dir)
            .map_err(|e| format!("não foi possível criar {}: {e}", dir.display()))?;
        let path = dir.join("daemon.log");
        if path.is_file() {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size >= MAX_SIZE {
                let backup = dir.join("daemon.log.1");
                let _ = fs::rename(&path, &backup);
            }
        }
        Ok(Self { path })
    }

    /// Appends `[timestamp] {line}\n` to the log. Silently ignores IO errors.
    pub fn write(&self, line: &str) {
        let ts = timestamp();
        let entry = format!("[{ts}] {line}\n");
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(entry.as_bytes());
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn timestamp() -> String {
    // RFC-3339 date-time without fractional seconds, e.g. "2026-10-29T14:23:05"
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = epoch_to_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

// Minimal epoch → calendar decomposition (no external crate).
fn epoch_to_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let days = hours / 24;
    // Days since 1970-01-01; use a simple Gregorian algorithm.
    let (y, mo, d) = days_to_ymd(days);
    (y, mo, d, h, mi, s)
}

/// Public alias used by the `logs` module for session date formatting.
pub fn days_to_ymd_pub(days: u64) -> (u64, u64, u64) {
    days_to_ymd(days)
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(suffix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("dcx-daemonlog-{}-{nanos}", suffix))
    }

    #[test]
    fn open_creates_directory_when_missing() {
        let home = tmp_dir("mkdir");
        assert!(!home.exists());
        let log = DaemonLog::open(&home).expect("open");
        assert!(home.join(".dcx").is_dir());
        let _ = fs::remove_dir_all(&home);
        drop(log);
    }

    #[test]
    fn write_appends_line_with_timestamp() {
        let home = tmp_dir("write");
        let log = DaemonLog::open(&home).expect("open");
        log.write("daemon started on 127.0.0.1:8787");
        let contents = fs::read_to_string(log.path()).expect("read");
        assert!(contents.contains("daemon started on 127.0.0.1:8787"));
        assert!(
            contents.starts_with('['),
            "should start with timestamp bracket"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rotation_renames_file_when_over_limit() {
        let home = tmp_dir("rotate");
        let log = DaemonLog::open(&home).expect("first open");
        // Write more than MAX_SIZE bytes into the log file directly.
        let big = vec![b'x'; (MAX_SIZE + 1) as usize];
        fs::write(log.path(), &big).expect("write big");
        // Re-open — should trigger rotation.
        let log2 = DaemonLog::open(&home).expect("second open");
        let backup = home.join(".dcx").join("daemon.log.1");
        assert!(backup.exists(), "daemon.log.1 should exist after rotation");
        // The new log should be empty (no writes yet) or very small.
        let new_size = fs::metadata(log2.path()).map(|m| m.len()).unwrap_or(0);
        assert!(new_size < MAX_SIZE, "rotated log should be fresh");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn multiple_writes_accumulate() {
        let home = tmp_dir("multi");
        let log = DaemonLog::open(&home).expect("open");
        log.write("line one");
        log.write("line two");
        let contents = fs::read_to_string(log.path()).expect("read");
        assert!(contents.contains("line one"));
        assert!(contents.contains("line two"));
        assert_eq!(contents.lines().count(), 2);
        let _ = fs::remove_dir_all(&home);
    }
}
