//! The `Host` trait: what the program observes on the host, and its real implementation.

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use crate::adb::device::Serial;
use crate::config::Config;
use crate::error::Result;
use crate::net::interface::{InterfaceName, InterfaceStatus, parse_global_ipv4};
use crate::net::probe::ping_args;
use crate::process::{Cmd, run};

/// Timeout for quick local queries (`ip`, `systemctl is-active`).
const LOCAL_TIMEOUT: Duration = Duration::from_secs(5);

/// Read-only view of the host. Faked in tests.
pub trait Host {
    /// Whether a systemd unit is active.
    fn unit_active(&self, unit: &str) -> Result<bool>;
    /// Presence, global IPv4 address and driver of an interface.
    fn interface(&self, name: &InterfaceName) -> Result<InterfaceStatus>;
    /// One ping through `name` to the configured probe host.
    fn probe(&self, name: &InterfaceName) -> Result<bool>;
    /// Whether a USB device with this serial is attached (seen by the
    /// kernel, regardless of ADB).
    fn usb_serial_present(&self, serial: &Serial) -> bool;
}

/// The real host, via `systemctl`, `ip`, `ping` and sysfs.
pub struct RealHost {
    ip: std::path::PathBuf,
    ping: std::path::PathBuf,
    systemctl: std::path::PathBuf,
    probe_host: Ipv4Addr,
    probe_timeout: Duration,
}

impl RealHost {
    /// A host using the configured tool paths and probe settings.
    pub fn new(config: &Config) -> Self {
        Self {
            ip: config.paths.ip.clone(),
            ping: config.paths.ping.clone(),
            systemctl: config.paths.systemctl.clone(),
            probe_host: config.wan.probe_host,
            probe_timeout: config.wan.probe_timeout_secs.get(),
        }
    }
}

impl Host for RealHost {
    fn unit_active(&self, unit: &str) -> Result<bool> {
        let cmd = Cmd::new(&self.systemctl, LOCAL_TIMEOUT).args(["is-active", "--quiet", unit]);
        Ok(run(&cmd)?.success())
    }

    fn interface(&self, name: &InterfaceName) -> Result<InterfaceStatus> {
        let sys = Path::new("/sys/class/net").join(name.as_str());
        if !sys.exists() {
            return Ok(InterfaceStatus::default());
        }
        let driver = std::fs::read_link(sys.join("device/driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let cmd = Cmd::new(&self.ip, LOCAL_TIMEOUT).args([
            "-4",
            "-o",
            "address",
            "show",
            "dev",
            name.as_str(),
            "scope",
            "global",
        ]);
        let output = run(&cmd)?;
        Ok(InterfaceStatus {
            present: true,
            ipv4: parse_global_ipv4(&output.stdout),
            driver,
        })
    }

    fn probe(&self, name: &InterfaceName) -> Result<bool> {
        let cmd = Cmd::new(&self.ping, self.probe_timeout + Duration::from_secs(2))
            .args(ping_args(name, self.probe_host, self.probe_timeout));
        Ok(run(&cmd)?.success())
    }

    fn usb_serial_present(&self, serial: &Serial) -> bool {
        usb_devices().iter().any(|d| d.serial == serial.as_str())
    }
}

/// A USB device as the kernel sees it in sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    /// Canonical sysfs path.
    pub path: std::path::PathBuf,
    /// iSerialNumber.
    pub serial: String,
    /// Vendor ID, hex.
    pub vendor: String,
    /// Product ID, hex.
    pub product: String,
    /// Whether it exposes an ADB interface (class ff, subclass 42, protocol 01).
    pub adb: bool,
}

/// All attached USB devices that report a serial.
pub fn usb_devices() -> Vec<UsbDevice> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let read = |p: &Path, f: &str| {
        std::fs::read_to_string(p.join(f))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = std::fs::canonicalize(entry.path()).ok()?;
            let serial = read(&path, "serial");
            if serial.is_empty() {
                return None;
            }
            let adb = std::fs::read_dir(&path).ok()?.flatten().any(|child| {
                let p = child.path();
                read(&p, "bInterfaceClass") == "ff"
                    && read(&p, "bInterfaceSubClass") == "42"
                    && read(&p, "bInterfaceProtocol") == "01"
            });
            Some(UsbDevice {
                vendor: read(&path, "idVendor"),
                product: read(&path, "idProduct"),
                path,
                serial,
                adb,
            })
        })
        .collect()
}

/// Network interfaces whose device sits under the USB device at `usb_path`.
pub fn interfaces_of_usb(usb_path: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            std::fs::canonicalize(entry.path().join("device"))
                .is_ok_and(|device| device.starts_with(usb_path))
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}
