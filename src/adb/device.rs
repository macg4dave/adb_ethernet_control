//! Device listing and selection by serial and current USB transport ID.

use std::fmt;

use serde::Deserialize;

/// A phone serial number, restricted to characters that are safe in
/// arguments and logs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Serial(String);

impl Serial {
    /// The serial as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The last four characters, for logs that should not expose the serial.
    pub fn redacted(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
        format!("…{tail}")
    }
}

impl TryFrom<String> for Serial {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b));
        if valid {
            Ok(Self(value))
        } else {
            Err(format!(
                "invalid phone serial {value:?}: use letters, digits and ._:- only"
            ))
        }
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// ADB's per-connection transport ID; changes on every USB re-enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportId(pub u32);

impl fmt::Display for TransportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// ADB connection state of a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    /// Authorized and usable.
    Device,
    /// The phone has not accepted this host's key.
    Unauthorized,
    /// Present but not responding.
    Offline,
    /// Any other state ADB reports.
    Other(String),
}

impl DeviceState {
    fn parse(word: &str) -> Self {
        match word {
            "device" => Self::Device,
            "unauthorized" => Self::Unauthorized,
            "offline" => Self::Offline,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device => f.write_str("device"),
            Self::Unauthorized => f.write_str("unauthorized"),
            Self::Offline => f.write_str("offline"),
            Self::Other(word) => f.write_str(word),
        }
    }
}

/// One line of `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    /// Serial as reported (untrusted text).
    pub serial: String,
    /// Connection state.
    pub state: DeviceState,
    /// Whether the line has a `usb:` field (not a TCP connection).
    pub usb: bool,
    /// The `transport_id:` field, if present.
    pub transport: Option<TransportId>,
}

/// Parses `adb devices -l`, skipping headers and daemon messages.
pub fn parse(listing: &str) -> Vec<DeviceEntry> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            if serial == "List" || serial.starts_with('*') || serial.starts_with("adb") {
                return None;
            }
            let state = DeviceState::parse(fields.next()?);
            let mut usb = false;
            let mut transport = None;
            for field in fields {
                if field.starts_with("usb:") {
                    usb = true;
                } else if let Some(id) = field.strip_prefix("transport_id:") {
                    transport = id.parse().ok().map(TransportId);
                }
            }
            Some(DeviceEntry {
                serial: serial.to_owned(),
                state,
                usb,
                transport,
            })
        })
        .collect()
}

/// Result of looking for the configured phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Exactly one authorized USB transport.
    Selected(TransportId),
    /// Not listed at all.
    Absent,
    /// Listed but not usable (unauthorized, offline, ...).
    NotReady(DeviceState),
    /// Only reachable over TCP, which is never used.
    NotUsb,
    /// Listed without a transport ID (very old ADB).
    NoTransportId,
    /// More than one usable USB transport for the same serial.
    Multiple,
}

impl Selection {
    /// Why the phone cannot be used, for logs; `None` when selected.
    pub fn problem(&self) -> Option<String> {
        match self {
            Self::Selected(_) => None,
            Self::Absent => Some("configured phone not listed by ADB".into()),
            Self::NotReady(DeviceState::Unauthorized) => {
                Some("phone is unauthorized: unlock it and accept \"Allow USB debugging\"".into())
            }
            Self::NotReady(state) => Some(format!("phone is {state}")),
            Self::NotUsb => Some("phone is only connected over TCP, not USB".into()),
            Self::NoTransportId => Some("ADB did not report a transport ID".into()),
            Self::Multiple => Some("phone has multiple USB transports".into()),
        }
    }
}

/// Picks the configured phone's single authorized USB transport.
pub fn select(entries: &[DeviceEntry], serial: &Serial) -> Selection {
    let mine: Vec<&DeviceEntry> = entries
        .iter()
        .filter(|e| e.serial == serial.as_str())
        .collect();
    if mine.is_empty() {
        return Selection::Absent;
    }
    let usable: Vec<&DeviceEntry> = mine
        .iter()
        .copied()
        .filter(|e| e.usb && e.state == DeviceState::Device)
        .collect();
    match usable.as_slice() {
        [] => {
            if let Some(entry) = mine.iter().find(|e| e.usb) {
                Selection::NotReady(entry.state.clone())
            } else {
                Selection::NotUsb
            }
        }
        [entry] => entry
            .transport
            .map_or(Selection::NoTransportId, Selection::Selected),
        _ => Selection::Multiple,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial(s: &str) -> Serial {
        Serial::try_from(s.to_owned()).unwrap()
    }

    const LISTING: &str = "List of devices attached\n\
        OTHER device usb:1-1 product:x model:y device:z transport_id:7\n\
        FAKE0SERIAL device usb:1-2 product:ALI model:ALI_NX1 device:HN transport_id:12\n";

    #[test]
    fn selects_configured_usb_phone_among_others() {
        let entries = parse(LISTING);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            select(&entries, &serial("FAKE0SERIAL")),
            Selection::Selected(TransportId(12))
        );
    }

    #[test]
    fn explicit_problems() {
        let cases = [
            ("", Selection::Absent),
            ("* daemon started successfully\n", Selection::Absent),
            (
                "FAKE0SERIAL unauthorized usb:1-2 transport_id:3\n",
                Selection::NotReady(DeviceState::Unauthorized),
            ),
            (
                "FAKE0SERIAL offline usb:1-2 transport_id:3\n",
                Selection::NotReady(DeviceState::Offline),
            ),
            ("FAKE0SERIAL device transport_id:3\n", Selection::NotUsb),
            ("FAKE0SERIAL device usb:1-2\n", Selection::NoTransportId),
            (
                "FAKE0SERIAL device usb:1-2 transport_id:3\nFAKE0SERIAL device usb:1-3 transport_id:4\n",
                Selection::Multiple,
            ),
        ];
        for (listing, expected) in cases {
            assert_eq!(
                select(&parse(listing), &serial("FAKE0SERIAL")),
                expected,
                "{listing:?}"
            );
        }
    }

    #[test]
    fn serial_validation_and_redaction() {
        assert!(Serial::try_from("A1b2.C3:4-5_6".to_owned()).is_ok());
        assert!(Serial::try_from(String::new()).is_err());
        assert!(Serial::try_from("bad serial".to_owned()).is_err());
        assert!(Serial::try_from("x;rm".to_owned()).is_err());
        assert_eq!(serial("ABCDEF123").redacted(), "…F123");
        assert_eq!(serial("AB").redacted(), "…AB");
    }
}
