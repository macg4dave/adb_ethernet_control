//! Markers in `/run/adb-wan/`: the once-per-boot guard and the "restore pending" flag.
//!
//! `/run` is emptied at every boot, which resets both.

use std::fs;
use std::path::PathBuf;

use crate::adb::usb_state::NetFunction;
use crate::error::{Error, Result};

const ACTED: &str = "acted-this-boot";
const RESTORE: &str = "restore-pending";

/// Marker files in the runtime directory.
pub struct Markers {
    dir: PathBuf,
}

impl Markers {
    /// Markers in `dir` (the unit's `RuntimeDirectory=`).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// What recovery action already ran this boot, if any.
    pub fn acted_this_boot(&self) -> Option<String> {
        fs::read_to_string(self.dir.join(ACTED))
            .ok()
            .map(|s| s.trim().to_owned())
    }

    /// Records that a recovery action is about to run. Written before the
    /// action, so a crash mid-action still counts.
    pub fn mark_acted(&self, action: &str) -> Result<()> {
        self.write(ACTED, action)
    }

    /// The function to re-enable if a disruptive step did not finish.
    pub fn restore_pending(&self) -> Option<NetFunction> {
        fs::read_to_string(self.dir.join(RESTORE))
            .ok()
            .and_then(|s| NetFunction::parse(s.trim()))
    }

    /// Arms the `ExecStopPost=` restore. Must succeed before USB goes off.
    pub fn set_restore_pending(&self, function: NetFunction) -> Result<()> {
        self.write(RESTORE, function.as_str())
    }

    /// Disarms the restore once USB Ethernet is observed on again.
    pub fn clear_restore_pending(&self) -> Result<()> {
        let path = self.dir.join(RESTORE);
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(Error::io(format!("removing {}", path.display()), e))
            }
            _ => Ok(()),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<()> {
        let path = self.dir.join(name);
        fs::create_dir_all(&self.dir)
            .and_then(|()| fs::write(&path, format!("{value}\n")))
            .map_err(|e| Error::io(format!("writing {}", path.display()), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip() {
        let dir = std::env::temp_dir().join(format!("adb-wan-markers-{}", std::process::id()));
        let markers = Markers::new(&dir);
        assert_eq!(markers.acted_this_boot(), None);
        assert_eq!(markers.restore_pending(), None);
        markers.mark_acted("cycle").unwrap();
        markers.set_restore_pending(NetFunction::Ncm).unwrap();
        assert_eq!(markers.acted_this_boot().as_deref(), Some("cycle"));
        assert_eq!(markers.restore_pending(), Some(NetFunction::Ncm));
        markers.clear_restore_pending().unwrap();
        markers.clear_restore_pending().unwrap();
        assert_eq!(markers.restore_pending(), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
