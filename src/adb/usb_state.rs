//! Parsing `sys.usb.state` into USB Ethernet on / off / unknown.

use std::fmt;

use serde::Deserialize;

/// The USB function that carries tethered Ethernet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetFunction {
    /// RNDIS (what the HONOR phone uses).
    Rndis,
    /// CDC NCM.
    Ncm,
}

impl NetFunction {
    /// The function name used by `svc usb setFunctions`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rndis => "rndis",
            Self::Ncm => "ncm",
        }
    }

    /// Parses a function name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "rndis" => Some(Self::Rndis),
            "ncm" => Some(Self::Ncm),
            _ => None,
        }
    }
}

impl fmt::Display for NetFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the phone's USB state says about tethered Ethernet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbEthernet {
    /// A networking function is active, alongside ADB.
    On(NetFunction),
    /// ADB is active without a networking function.
    Off,
    /// The state cannot be trusted; the reason says why. Never treat as off.
    Unknown(String),
}

impl UsbEthernet {
    /// `on`, `off` or `unknown`, for summaries.
    pub fn word(&self) -> &'static str {
        match self {
            Self::On(_) => "on",
            Self::Off => "off",
            Self::Unknown(_) => "unknown",
        }
    }
}

impl fmt::Display for UsbEthernet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::On(function) => write!(f, "on({function})"),
            Self::Off => f.write_str("off"),
            Self::Unknown(reason) => write!(f, "unknown({reason})"),
        }
    }
}

/// A raw `sys.usb.state` value and its interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbState {
    /// The value as read, with whitespace trimmed.
    pub raw: String,
    /// The interpretation.
    pub ethernet: UsbEthernet,
}

/// Interprets `sys.usb.state` as a set of functions. Vendors add their own
/// functions (HONOR boots as `hisuite,mtp,mass_storage,adb`), so only the
/// presence of `adb`, `rndis` and `ncm` matters.
pub fn parse(raw: &str) -> UsbState {
    let raw = raw.trim().to_owned();
    let ethernet = interpret(&raw);
    UsbState { raw, ethernet }
}

fn interpret(raw: &str) -> UsbEthernet {
    if raw.is_empty() {
        return UsbEthernet::Unknown("empty".into());
    }
    let functions: Vec<&str> = raw.split(',').collect();
    let well_formed = functions.iter().all(|f| {
        !f.is_empty()
            && f.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    });
    if !well_formed {
        return UsbEthernet::Unknown("malformed".into());
    }
    if !functions.contains(&"adb") {
        return UsbEthernet::Unknown("adb function missing".into());
    }
    let networking: Vec<NetFunction> = functions
        .iter()
        .filter_map(|f| NetFunction::parse(f))
        .collect();
    match networking.as_slice() {
        [] => UsbEthernet::Off,
        [function] => UsbEthernet::On(*function),
        _ => UsbEthernet::Unknown("both rndis and ncm".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eth(raw: &str) -> UsbEthernet {
        parse(raw).ethernet
    }

    #[test]
    fn tethering_on() {
        assert_eq!(eth("rndis,adb"), UsbEthernet::On(NetFunction::Rndis));
        assert_eq!(eth("adb,ncm\r\n"), UsbEthernet::On(NetFunction::Ncm));
    }

    #[test]
    fn vendor_boot_sets_are_off() {
        assert_eq!(eth("hisuite,mtp,mass_storage,adb"), UsbEthernet::Off);
        assert_eq!(eth("mtp,adb,hdb"), UsbEthernet::Off);
        assert_eq!(eth("mtp,adb"), UsbEthernet::Off);
    }

    #[test]
    fn untrustworthy_states_are_unknown() {
        for raw in [
            "",
            "  ",
            "rndis",
            "mtp",
            "RNDIS,adb",
            "rndis,,adb",
            "rndis adb",
            "rndis,ncm,adb",
        ] {
            assert!(
                matches!(eth(raw), UsbEthernet::Unknown(_)),
                "{raw:?} must be unknown"
            );
        }
    }

    #[test]
    fn raw_is_trimmed() {
        assert_eq!(parse("rndis,adb\r\n").raw, "rndis,adb");
    }
}
