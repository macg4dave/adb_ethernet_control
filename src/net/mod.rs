//! Host-side network observation: interface, address, probe, and WAN health.

pub mod interface;
pub mod network_manager;
pub mod probe;

use std::fmt;

use crate::error::Result;
use crate::host::Host;
use interface::InterfaceName;

/// Result of one WAN health check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WanHealth {
    /// Global IPv4 address present and the bound probe answered.
    Up {
        /// The address with prefix.
        address: String,
    },
    /// The interface does not exist (phone exposes no USB Ethernet).
    NoInterface,
    /// The interface exists but has no global IPv4 address.
    NoAddress,
    /// Address present but the probe got no answer.
    ProbeFailed {
        /// The address with prefix.
        address: String,
    },
}

impl WanHealth {
    /// Whether WAN works.
    pub fn is_up(&self) -> bool {
        matches!(self, Self::Up { .. })
    }
}

impl fmt::Display for WanHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Up { address } => write!(f, "up address={address}"),
            Self::NoInterface => f.write_str("down reason=no-interface"),
            Self::NoAddress => f.write_str("down reason=no-ipv4-address"),
            Self::ProbeFailed { address } => {
                write!(f, "down reason=probe-failed address={address}")
            }
        }
    }
}

/// Checks WAN health on `interface`: address first, then the bound probe.
pub fn wan_health(host: &dyn Host, interface: &InterfaceName) -> Result<WanHealth> {
    let status = host.interface(interface)?;
    if !status.present {
        return Ok(WanHealth::NoInterface);
    }
    let Some(address) = status.ipv4 else {
        return Ok(WanHealth::NoAddress);
    };
    if host.probe(interface)? {
        Ok(WanHealth::Up { address })
    } else {
        Ok(WanHealth::ProbeFailed { address })
    }
}
