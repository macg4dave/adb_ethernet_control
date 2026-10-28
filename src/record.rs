//! Persistent per-run records (`result` + `events`) under `/var/log/adb-wan/`.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

/// One run's record directory.
pub struct Record {
    dir: PathBuf,
}

impl Record {
    /// Creates `<log_dir>/<kind>-<UTC time>/` with `status=running`.
    pub fn create(log_dir: &Path, kind: &str) -> Result<Self> {
        let stamp = utc_now().replace(':', "");
        let mut dir = log_dir.join(format!("{kind}-{stamp}"));
        let mut n = 1;
        while dir.exists() {
            n += 1;
            dir = log_dir.join(format!("{kind}-{stamp}-{n}"));
        }
        fs::create_dir_all(&dir)
            .map_err(|e| Error::io(format!("creating {}", dir.display()), e))?;
        let record = Self { dir };
        record.write_result(&[("status", "running".into())])?;
        Ok(record)
    }

    /// The record directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Opens the `events` file for the log to append to.
    pub fn events(&self) -> Result<File> {
        let path = self.dir.join("events");
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| Error::io(format!("opening {}", path.display()), e))
    }

    /// Atomically replaces the `result` file with `key=value` lines plus
    /// `updated=<UTC time>`.
    pub fn write_result(&self, fields: &[(&str, String)]) -> Result<()> {
        let mut text = String::new();
        for (key, value) in fields {
            // One line per field even if a value (phone output) has newlines.
            let value = value.replace('\n', "\\n");
            let _ = writeln!(text, "{key}={value}");
        }
        let _ = writeln!(text, "updated={}", utc_now());
        let tmp = self.dir.join("result.tmp");
        let path = self.dir.join("result");
        let write = || -> std::io::Result<()> {
            let mut file = File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, &path)
        };
        write().map_err(|e| Error::io(format!("writing {}", path.display()), e))
    }
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_utc(secs)
}

/// Formats seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm, for dates after 1970.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// The kernel's random ID for this boot.
pub fn boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_or_else(|_| "unknown".into(), |s| s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(1_793_219_697), "2026-10-28T20:34:57Z");
    }

    #[test]
    fn record_lifecycle() {
        let dir = std::env::temp_dir().join(format!("adb-wan-record-{}", std::process::id()));
        let record = Record::create(&dir, "boot").unwrap();
        let result = fs::read_to_string(record.dir().join("result")).unwrap();
        assert!(result.starts_with("status=running\n"));
        record
            .write_result(&[("status", "passed".into()), ("exit_code", "0".into())])
            .unwrap();
        let result = fs::read_to_string(record.dir().join("result")).unwrap();
        assert!(result.starts_with("status=passed\nexit_code=0\nupdated="));
        let second = Record::create(&dir, "boot").unwrap();
        assert_ne!(second.dir(), record.dir());
        fs::remove_dir_all(dir).unwrap();
    }
}
