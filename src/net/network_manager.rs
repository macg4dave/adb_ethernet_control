//! Read-only NetworkManager view of the WAN interface (never changes profiles).

use std::fmt;

use super::interface::InterfaceName;

/// NetworkManager's state for one device (`GENERAL.STATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmState {
    /// NetworkManager does not know the device (interface absent).
    Absent,
    /// Preparing, configuring, getting an address, or deactivating:
    /// NetworkManager has not finished; wait for it.
    Busy(u16),
    /// Activated (100).
    Connected,
    /// Activation failed (120).
    Failed,
    /// Unmanaged, unavailable or disconnected (10/20/30).
    Idle(u16),
}

impl NmState {
    /// Whether NetworkManager has finished with the device: connected or
    /// failed. Idle is not finished: autoconnect may start any moment.
    pub fn finished(self) -> bool {
        matches!(self, Self::Connected | Self::Failed)
    }
}

impl fmt::Display for NmState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("absent"),
            Self::Busy(code) => write!(f, "busy({code})"),
            Self::Connected => f.write_str("connected"),
            Self::Failed => f.write_str("failed"),
            Self::Idle(code) => write!(f, "idle({code})"),
        }
    }
}

/// Parses `nmcli -g GENERAL.STATE device show <if>`, e.g. `100 (connected)`.
pub fn parse_state(output: &str) -> Option<NmState> {
    let code: u16 = output.split_whitespace().next()?.parse().ok()?;
    Some(match code {
        100 => NmState::Connected,
        120 => NmState::Failed,
        40..=90 | 110 => NmState::Busy(code),
        other => NmState::Idle(other),
    })
}

/// `nmcli` arguments that print only the device state.
pub fn state_args(interface: &InterfaceName) -> Vec<String> {
    vec![
        "-g".into(),
        "GENERAL.STATE".into(),
        "device".into(),
        "show".into(),
        interface.as_str().into(),
    ]
}

/// `nmcli` arguments that print the interface's state, profile and driver.
pub fn device_show_args(interface: &InterfaceName) -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "GENERAL.STATE,GENERAL.CONNECTION,GENERAL.DRIVER".into(),
        "device".into(),
        "show".into(),
        interface.as_str().into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_states() {
        assert_eq!(parse_state("100 (connected)\n"), Some(NmState::Connected));
        assert_eq!(
            parse_state("70 (connecting (getting IP configuration))"),
            Some(NmState::Busy(70))
        );
        assert_eq!(parse_state("110 (deactivating)"), Some(NmState::Busy(110)));
        assert_eq!(parse_state("120 (failed)"), Some(NmState::Failed));
        assert_eq!(parse_state("30 (disconnected)"), Some(NmState::Idle(30)));
        assert_eq!(parse_state(""), None);
        assert_eq!(parse_state("garbage"), None);
    }

    #[test]
    fn only_connected_or_failed_is_finished() {
        assert!(NmState::Connected.finished());
        assert!(NmState::Failed.finished());
        assert!(!NmState::Busy(70).finished());
        assert!(!NmState::Idle(30).finished());
        assert!(!NmState::Absent.finished());
    }
}
