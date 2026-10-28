//! Fixed safety limits. These are not tuning knobs and not settings: every
//! wait ends as soon as the event it waits for happens. A limit is only
//! reached when that event never comes, so nothing can hang forever.

use std::time::Duration;

/// Longest any single wait may last (phone appears, USB state changes,
/// NetworkManager finishes, WAN answers).
pub const WAIT: Duration = Duration::from_secs(120);
/// How often a waited-for condition is re-checked.
pub const POLL: Duration = Duration::from_secs(1);
/// One `adb` client command.
pub const ADB_COMMAND: Duration = Duration::from_secs(10);
/// Our ADB server answering after it is started.
pub const ADB_SERVER_START: Duration = Duration::from_secs(15);
/// One ping.
pub const PROBE: Duration = Duration::from_secs(5);
/// Quick local queries (`ip`, `systemctl is-active`, `nmcli`).
pub const LOCAL_COMMAND: Duration = Duration::from_secs(10);

/// Waits in the longest boot path: phone, USB state, off, on, WAN.
const BOOT_WAITS: u32 = 5;

/// Upper bound of one boot run; must stay inside the unit's
/// `TimeoutStartSec=15min`.
pub fn boot_worst_case() -> Duration {
    ADB_SERVER_START + WAIT * BOOT_WAITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_fits_inside_the_unit_timeout() {
        assert!(boot_worst_case() < Duration::from_secs(13 * 60));
    }
}
