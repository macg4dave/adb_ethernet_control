//! Read-only NetworkManager view of the WAN interface for `status` (never changes profiles).

use super::interface::InterfaceName;

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
