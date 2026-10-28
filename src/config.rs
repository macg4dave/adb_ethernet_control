//! The single TOML config file: typed, `deny_unknown_fields`, defaults and validation, loaded once.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::adb::device::Serial;
use crate::adb::usb_state::NetFunction;
use crate::error::{Error, Result};
use crate::net::interface::InterfaceName;

/// Where the installed config lives.
pub const DEFAULT_PATH: &str = "/etc/adb-ethernet-control/config.toml";

/// The committed example, also the template `install` fills in.
pub const EXAMPLE: &str = include_str!("../config/config.example.toml");

/// All settings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// `[phone]`
    pub phone: Phone,
    /// `[wan]`
    pub wan: Wan,
    /// `[adb]`
    #[serde(default)]
    pub adb: Adb,
    /// `[readiness]`
    #[serde(default)]
    pub readiness: Readiness,
    /// `[recovery]`
    #[serde(default)]
    pub recovery: Recovery,
    /// `[paths]`
    #[serde(default)]
    pub paths: Paths,
}

/// `[phone]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phone {
    /// USB serial of the tethering phone.
    pub serial: Serial,
    /// Function enabled when USB Ethernet is off.
    #[serde(default = "default_usb_function")]
    pub usb_function: NetFunction,
}

/// `[wan]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wan {
    /// Host interface of the phone's USB Ethernet.
    pub interface: InterfaceName,
    /// Ping target.
    #[serde(default = "default_probe_host")]
    pub probe_host: Ipv4Addr,
}

/// `[adb]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Adb {
    /// Path to the `adb` executable.
    pub binary: PathBuf,
    /// Loopback port of our own ADB server.
    pub server_port: u16,
    /// Start the server with the libusb backend.
    pub libusb: bool,
    /// HOME of the ADB server (holds the authorized key).
    pub home: PathBuf,
}

impl Default for Adb {
    fn default() -> Self {
        Self {
            binary: "/usr/local/bin/adb".into(),
            server_port: 5038,
            libusb: true,
            home: "/var/lib/adb-wan".into(),
        }
    }
}

/// `[readiness]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Readiness {
    /// Services `adb-wan.service` is ordered after (`install` writes the
    /// `After=` drop-in).
    pub units: Vec<String>,
}

impl Default for Readiness {
    fn default() -> Self {
        Self {
            units: vec!["NetworkManager.service".into()],
        }
    }
}

/// `[recovery]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Recovery {
    /// Allow one off/on cycle when USB Ethernet is on but WAN is down.
    pub usb_cycle_enabled: bool,
    /// Function set used for "off"; must keep ADB alive.
    pub off_function: String,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            usb_cycle_enabled: false,
            off_function: "mtp".into(),
        }
    }
}

/// `[paths]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Paths {
    /// `ip` executable.
    pub ip: PathBuf,
    /// `ping` executable.
    pub ping: PathBuf,
    /// `systemctl` executable.
    pub systemctl: PathBuf,
    /// `nmcli` executable (read-only use in `status`).
    pub nmcli: PathBuf,
    /// Per-run records.
    pub log_dir: PathBuf,
    /// Once-per-boot and restore-pending markers.
    pub runtime_dir: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            ip: "/usr/sbin/ip".into(),
            ping: "/usr/bin/ping".into(),
            systemctl: "/usr/bin/systemctl".into(),
            nmcli: "/usr/bin/nmcli".into(),
            log_dir: "/var/log/adb-wan".into(),
            runtime_dir: "/run/adb-wan".into(),
        }
    }
}

fn default_usb_function() -> NetFunction {
    NetFunction::Rndis
}

fn default_probe_host() -> Ipv4Addr {
    Ipv4Addr::new(8, 8, 8, 8)
}

impl Config {
    /// Reads, parses and validates the config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;
        Self::parse(&text).map_err(|e| match e {
            Error::Config(message) => Error::Config(format!("{}: {message}", path.display())),
            other => other,
        })
    }

    /// Parses and validates config text.
    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.readiness.units.is_empty() {
            return Err(Error::Config(
                "readiness.units must list at least one service".into(),
            ));
        }
        for unit in &self.readiness.units {
            let valid = unit.ends_with(".service")
                && unit
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.@:-".contains(&b));
            if !valid {
                return Err(Error::Config(format!(
                    "readiness.units: invalid unit {unit:?}"
                )));
            }
        }
        let off = &self.recovery.off_function;
        let off_valid = !off.is_empty()
            && off
                .split(',')
                .all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
            && !off
                .split(',')
                .any(|f| NetFunction::parse(f).is_some() || f == "adb");
        if !off_valid {
            return Err(Error::Config(format!(
                "recovery.off_function {off:?} must be a non-networking function such as \"mtp\" (without adb)"
            )));
        }
        if self.adb.server_port < 1024 {
            return Err(Error::Config(
                "adb.server_port must be 1024 or higher".into(),
            ));
        }
        Ok(())
    }
}

/// Fills the example template with install answers. Only the marked lines
/// change, so the installed file keeps the example's comments.
pub fn render(serial: &Serial, interface: &InterfaceName, adb: &Path, units: &[String]) -> String {
    let units = units
        .iter()
        .map(|u| format!("\"{u}\""))
        .collect::<Vec<_>>()
        .join(", ");
    EXAMPLE
        .replace(
            "serial = \"REPLACE_WITH_USB_SERIAL\"",
            &format!("serial = \"{serial}\""),
        )
        .replace(
            "interface = \"REPLACE_WITH_INTERFACE\"",
            &format!("interface = \"{interface}\""),
        )
        .replace(
            "binary = \"/usr/local/bin/adb\"",
            &format!("binary = \"{}\"", adb.display()),
        )
        .replace(
            "units = [\"NetworkManager.service\"]",
            &format!("units = [{units}]"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "[phone]\nserial = \"FAKE0SERIAL\"\n[wan]\ninterface = \"usb0\"\n";

    fn filled_example() -> String {
        EXAMPLE
            .replace("REPLACE_WITH_USB_SERIAL", "FAKE0SERIAL")
            .replace("REPLACE_WITH_INTERFACE", "usb0")
    }

    #[test]
    fn example_documents_exactly_the_defaults() {
        let example = Config::parse(&filled_example()).unwrap();
        let minimal = Config::parse(MINIMAL).unwrap();
        assert_eq!(example, minimal);
    }

    #[test]
    fn unfilled_example_is_rejected() {
        assert!(Config::parse(EXAMPLE).is_err());
    }

    #[test]
    fn defaults_are_safe() {
        let config = Config::parse(MINIMAL).unwrap();
        assert!(!config.recovery.usb_cycle_enabled);
        assert_eq!(config.recovery.off_function, "mtp");
        assert_eq!(config.adb.server_port, 5038);
    }

    #[test]
    fn rejects_bad_values_with_key_names() {
        let cases = [
            (
                format!("{MINIMAL}[timing]\npoll_interval_secs = 2\n"),
                "timing",
            ),
            (format!("{MINIMAL}[adb]\nbogus = 1\n"), "bogus"),
            (
                format!("{MINIMAL}[recovery]\noff_function = \"\"\n"),
                "off_function",
            ),
            (
                format!("{MINIMAL}[recovery]\noff_function = \"rndis\"\n"),
                "off_function",
            ),
            (
                format!("{MINIMAL}[recovery]\noff_function = \"mtp,adb\"\n"),
                "off_function",
            ),
            (
                format!("{MINIMAL}[readiness]\nunits = []\n"),
                "readiness.units",
            ),
            (
                format!("{MINIMAL}[readiness]\nunits = [\"x; rm\"]\n"),
                "invalid unit",
            ),
            (
                "[phone]\nserial = \"a b\"\n[wan]\ninterface = \"usb0\"\n".into(),
                "serial",
            ),
            (MINIMAL.replace("usb0", "toolonginterface0"), "interface"),
        ];
        for (text, needle) in cases {
            let message = Config::parse(&text).unwrap_err().to_string();
            assert!(message.contains(needle), "{needle:?} not in {message:?}");
        }
    }

    #[test]
    fn render_fills_template_and_parses() {
        let serial = Serial::try_from("FAKE0SERIAL".to_owned()).unwrap();
        let iface = InterfaceName::try_from("usb0".to_owned()).unwrap();
        let units = vec!["NetworkManager.service".into(), "lan-dns.service".into()];
        let text = render(&serial, &iface, Path::new("/opt/adb"), &units);
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.phone.serial, serial);
        assert_eq!(config.wan.interface, iface);
        assert_eq!(config.adb.binary, PathBuf::from("/opt/adb"));
        assert_eq!(config.readiness.units, units);
        assert!(text.contains("# Keep false until"));
    }
}
