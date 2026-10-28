//! Pure decision logic: observed state and config in, one bounded action out. No I/O.

use std::fmt;

use crate::adb::usb_state::{NetFunction, UsbEthernet};

/// What boot recovery will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Request this networking function once.
    Enable(NetFunction),
    /// Switch off (keeping ADB) and back on to this function once.
    Cycle(NetFunction),
    /// Change nothing; the reason is logged.
    NoAction(&'static str),
}

impl Decision {
    /// `enable`, `cycle` or `none`, for summaries.
    pub fn action_word(&self) -> &'static str {
        match self {
            Self::Enable(_) => "enable",
            Self::Cycle(_) => "cycle",
            Self::NoAction(_) => "none",
        }
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enable(function) => write!(f, "enable function={function}"),
            Self::Cycle(function) => write!(f, "cycle function={function}"),
            Self::NoAction(reason) => write!(f, "none reason={reason:?}"),
        }
    }
}

/// Decides the single recovery action for an unhealthy WAN.
pub fn decide(
    usb: &UsbEthernet,
    configured: NetFunction,
    cycle_enabled: bool,
    acted_this_boot: bool,
) -> Decision {
    if acted_this_boot {
        return Decision::NoAction("a recovery action already ran this boot");
    }
    match usb {
        UsbEthernet::Off => Decision::Enable(configured),
        UsbEthernet::On(active) if cycle_enabled => Decision::Cycle(*active),
        UsbEthernet::On(_) => {
            Decision::NoAction("USB Ethernet is on but recovery.usb_cycle_enabled is false")
        }
        UsbEthernet::Unknown(_) => Decision::NoAction("USB state is unknown; never act on it"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use NetFunction::{Ncm, Rndis};

    #[test]
    fn off_is_enabled_with_configured_function() {
        assert_eq!(
            decide(&UsbEthernet::Off, Ncm, false, false),
            Decision::Enable(Ncm)
        );
    }

    #[test]
    fn on_cycles_only_when_enabled_and_keeps_active_function() {
        assert_eq!(
            decide(&UsbEthernet::On(Ncm), Rndis, true, false),
            Decision::Cycle(Ncm)
        );
        assert!(matches!(
            decide(&UsbEthernet::On(Rndis), Rndis, false, false),
            Decision::NoAction(_)
        ));
    }

    #[test]
    fn unknown_and_already_acted_never_act() {
        let unknown = UsbEthernet::Unknown("empty".into());
        assert!(matches!(
            decide(&unknown, Rndis, true, false),
            Decision::NoAction(_)
        ));
        for usb in [UsbEthernet::Off, UsbEthernet::On(Rndis)] {
            assert!(matches!(
                decide(&usb, Rndis, true, true),
                Decision::NoAction(_)
            ));
        }
    }
}
