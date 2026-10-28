//! WAN interface name, presence and global IPv4 address checks.

use std::fmt;

use serde::Deserialize;

/// A Linux network interface name (at most 15 safe characters).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct InterfaceName(String);

impl InterfaceName {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for InterfaceName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = !value.is_empty()
            && value.len() <= 15
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b));
        if valid {
            Ok(Self(value))
        } else {
            Err(format!("invalid interface name {value:?}"))
        }
    }
}

impl fmt::Display for InterfaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the host currently sees on the WAN interface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterfaceStatus {
    /// The interface exists (the phone exposes USB Ethernet).
    pub present: bool,
    /// Its global IPv4 address with prefix, if any.
    pub ipv4: Option<String>,
    /// Kernel driver, e.g. `rndis_host` or `cdc_ncm`.
    pub driver: Option<String>,
}

/// Extracts the first global IPv4 address from
/// `ip -4 -o address show dev <if> scope global`.
pub fn parse_global_ipv4(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        fields.find(|f| *f == "inet")?;
        let address = fields.next()?;
        let ip = address.split('/').next()?;
        ip.parse::<std::net::Ipv4Addr>().ok()?;
        Some(address.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ip_output() {
        let out = "5: enp48s0f4u2    inet 192.168.42.17/24 brd 192.168.42.255 scope global dynamic noprefixroute enp48s0f4u2\\       valid_lft 3599sec preferred_lft 3599sec\n";
        assert_eq!(parse_global_ipv4(out), Some("192.168.42.17/24".into()));
        assert_eq!(parse_global_ipv4(""), None);
        assert_eq!(parse_global_ipv4("5: x inet garbage scope global\n"), None);
    }

    #[test]
    fn interface_name_validation() {
        assert!(InterfaceName::try_from("enp48s0f4u2".to_owned()).is_ok());
        assert!(InterfaceName::try_from(String::new()).is_err());
        assert!(InterfaceName::try_from("a_very_long_interface".to_owned()).is_err());
        assert!(InterfaceName::try_from("eth0;ls".to_owned()).is_err());
    }
}
