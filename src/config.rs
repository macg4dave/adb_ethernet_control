//! The single TOML config file: typed, `deny_unknown_fields`, defaults and validation, loaded once.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::adb::device::Serial;
use crate::adb::usb_state::NetFunction;
use crate::error::{Error, Result};
use crate::net::interface::InterfaceName;

/// Where the installed config lives.
pub const DEFAULT_PATH: &str = "/etc/adb-ethernet-control/config.toml";

/// The committed example, also the template `install` fills in.
pub const EXAMPLE: &str = include_str!("../config/config.example.toml");

/// The whole run must finish well inside the unit's `TimeoutStartSec=15min`.
const MAX_TOTAL_WAIT: Duration = Duration::from_secs(13 * 60);

/// A whole number of seconds between 1 and 3600.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "u64")]
pub struct Secs(Duration);

impl Secs {
    /// The value as a duration.
    pub fn get(self) -> Duration {
        self.0
    }

    const fn of(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }
}

impl TryFrom<u64> for Secs {
    type Error = String;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        if (1..=3600).contains(&value) {
            Ok(Self::of(value))
        } else {
            Err(format!("{value} seconds is out of range 1..=3600"))
        }
    }
}

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
    /// `[timing]`
    #[serde(default)]
    pub timing: Timing,
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
    /// Ping timeout.
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout_secs: Secs,
}

/// `[adb]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Adb {
    /// Path to the `adb` executable.
    pub binary: PathBuf,
    /// Loopback port of our own ADB server.
    pub server_port: u16,
    /// Timeout of each ADB client command.
    pub command_timeout_secs: Secs,
    /// How long the server may take to answer after starting.
    pub server_start_timeout_secs: Secs,
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
            command_timeout_secs: Secs::of(10),
            server_start_timeout_secs: Secs::of(15),
            libusb: true,
            home: "/var/lib/adb-wan".into(),
        }
    }
}

/// `[readiness]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Readiness {
    /// Services that must be active before WAN is judged.
    pub units: Vec<String>,
    /// How long to wait for them.
    pub timeout_secs: Secs,
}

impl Default for Readiness {
    fn default() -> Self {
        Self {
            units: vec!["NetworkManager.service".into()],
            timeout_secs: Secs::of(120),
        }
    }
}

/// `[timing]`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Timing {
    /// Re-check interval of every wait.
    pub poll_interval_secs: Secs,
    /// How long WAN may take to come up on its own after readiness.
    pub initial_wan_timeout_secs: Secs,
    /// How long to wait for the phone to be listed and authorized.
    pub phone_timeout_secs: Secs,
    /// How long to wait for a valid or changed USB state.
    pub usb_state_timeout_secs: Secs,
    /// How long WAN may take to work after a recovery action.
    pub wan_timeout_secs: Secs,
    /// Interval between the two final WAN checks.
    pub confirm_interval_secs: Secs,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll_interval_secs: Secs::of(2),
            initial_wan_timeout_secs: Secs::of(90),
            phone_timeout_secs: Secs::of(60),
            usb_state_timeout_secs: Secs::of(30),
            wan_timeout_secs: Secs::of(60),
            confirm_interval_secs: Secs::of(10),
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

fn default_probe_timeout() -> Secs {
    Secs::of(5)
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
        if self.timing.poll_interval_secs.get() > Duration::from_secs(60) {
            return Err(Error::Config(
                "timing.poll_interval_secs must be at most 60".into(),
            ));
        }
        let total = self.worst_case_run();
        if total > MAX_TOTAL_WAIT {
            return Err(Error::Config(format!(
                "deadlines add up to {}s; keep them under {}s so the unit's 15 minute limit is never hit",
                total.as_secs(),
                MAX_TOTAL_WAIT.as_secs()
            )));
        }
        Ok(())
    }

    /// Upper bound of one boot run's waits, used to keep it inside the unit
    /// timeout.
    pub fn worst_case_run(&self) -> Duration {
        let t = &self.timing;
        let phone_and_state = t.phone_timeout_secs.get() + t.usb_state_timeout_secs.get();
        self.readiness.timeout_secs.get()
            + t.initial_wan_timeout_secs.get()
            + self.adb.server_start_timeout_secs.get()
            + phone_and_state
            // off then on, each with re-selection and a state wait
            + phone_and_state * 2
            + t.wan_timeout_secs.get()
            + t.confirm_interval_secs.get()
            + self.wan.probe_timeout_secs.get() * 2
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
        assert!(config.worst_case_run() <= MAX_TOTAL_WAIT);
    }

    #[test]
    fn rejects_bad_values_with_key_names() {
        let cases = [
            (format!("{MINIMAL}[timing]\nbogus = 1\n"), "bogus"),
            (
                format!("{MINIMAL}[timing]\nphone_timeout_secs = 0\n"),
                "out of range",
            ),
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
                format!("{MINIMAL}[timing]\nwan_timeout_secs = 3600\n"),
                "add up to",
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
