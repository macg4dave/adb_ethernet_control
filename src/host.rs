//! The `Host` trait: what the program observes on the host, and its real implementation.

use std::net::Ipv4Addr;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use crate::config::Config;
use crate::error::Result;
use crate::limits;
use crate::net::interface::{InterfaceName, InterfaceStatus, parse_global_ipv4};
use crate::net::network_manager::{NmState, parse_state, state_args};
use crate::net::probe::ping_args;
use crate::process::{Cmd, run};
use crate::usb::UsbDevice;

/// Read-only view of the host. Faked in tests.
pub trait Host {
    /// Whether a systemd unit is active.
    fn unit_active(&self, unit: &str) -> Result<bool>;
    /// Presence, global IPv4 address and driver of an interface.
    fn interface(&self, name: &InterfaceName) -> Result<InterfaceStatus>;
    /// NetworkManager's state for the interface.
    fn nm_state(&self, name: &InterfaceName) -> Result<NmState>;
    /// One ping through `name` to the configured probe host.
    fn probe(&self, name: &InterfaceName) -> Result<bool>;
    /// Every non-hub USB device the kernel sees, regardless of ADB (an
    /// lsusb-style scan with ports). For diagnostics.
    fn usb_scan(&self) -> Vec<UsbDevice>;
    /// Whether something listens on this loopback TCP port (e.g. a desktop ADB
    /// server on 5037 that may hold the phone). For diagnostics.
    fn tcp_listening(&self, port: u16) -> bool;
}

/// The real host, via `systemctl`, `ip`, `nmcli`, `ping` and sysfs.
pub struct RealHost {
    ip: std::path::PathBuf,
    ping: std::path::PathBuf,
    systemctl: std::path::PathBuf,
    nmcli: std::path::PathBuf,
    probe_host: Ipv4Addr,
}

impl RealHost {
    /// A host using the configured tool paths and probe settings.
    pub fn new(config: &Config) -> Self {
        Self {
            ip: config.paths.ip.clone(),
            ping: config.paths.ping.clone(),
            systemctl: config.paths.systemctl.clone(),
            nmcli: config.paths.nmcli.clone(),
            probe_host: config.wan.probe_host,
        }
    }
}

impl Host for RealHost {
    fn unit_active(&self, unit: &str) -> Result<bool> {
        let cmd =
            Cmd::new(&self.systemctl, limits::LOCAL_COMMAND).args(["is-active", "--quiet", unit]);
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
        let cmd = Cmd::new(&self.ip, limits::LOCAL_COMMAND).args([
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

    fn nm_state(&self, name: &InterfaceName) -> Result<NmState> {
        let output = run(&Cmd::new(&self.nmcli, limits::LOCAL_COMMAND).args(state_args(name)))?;
        // nmcli exits 10 for an unknown device.
        if !output.success() {
            return Ok(NmState::Absent);
        }
        Ok(parse_state(&output.stdout).unwrap_or(NmState::Absent))
    }

    fn probe(&self, name: &InterfaceName) -> Result<bool> {
        let cmd = Cmd::new(&self.ping, limits::PROBE + Duration::from_secs(2)).args(ping_args(
            name,
            self.probe_host,
            limits::PROBE,
        ));
        Ok(run(&cmd)?.success())
    }

    fn usb_scan(&self) -> Vec<UsbDevice> {
        usb_devices()
    }

    fn tcp_listening(&self, port: u16) -> bool {
        std::fs::read_to_string("/proc/net/tcp").is_ok_and(|table| tcp_table_listens(&table, port))
    }
}

/// All attached USB devices except hubs, sorted by port.
pub fn usb_devices() -> Vec<UsbDevice> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return Vec::new();
    };
    let read = |p: &Path, f: &str| {
        std::fs::read_to_string(p.join(f))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    let mut devices: Vec<UsbDevice> = entries
        .flatten()
        .filter_map(|entry| {
            let port = entry.file_name().to_string_lossy().into_owned();
            let path = std::fs::canonicalize(entry.path()).ok()?;
            // Devices only (interfaces have no idVendor); skip root hubs and hubs.
            if !path.join("idVendor").exists()
                || port.starts_with("usb")
                || read(&path, "bDeviceClass") == "09"
            {
                return None;
            }
            let mut interfaces: Vec<String> = std::fs::read_dir(&path)
                .ok()?
                .flatten()
                .map(|child| child.path())
                .filter(|p| p.join("bInterfaceClass").exists())
                .map(|p| {
                    format!(
                        "{}{}{}",
                        read(&p, "bInterfaceClass"),
                        read(&p, "bInterfaceSubClass"),
                        read(&p, "bInterfaceProtocol")
                    )
                })
                .collect();
            interfaces.sort();
            let adb = interfaces.iter().any(|i| i == "ff4201");
            let node = format!(
                "/dev/bus/usb/{:03}/{:03}",
                read(&path, "busnum").parse::<u32>().unwrap_or(0),
                read(&path, "devnum").parse::<u32>().unwrap_or(0)
            );
            let access = std::fs::metadata(&node).map_or_else(
                |e| format!("unreadable({e})"),
                |m| {
                    format!(
                        "owner={}:{} mode={:04o}",
                        m.uid(),
                        m.gid(),
                        m.mode() & 0o7777
                    )
                },
            );
            Some(UsbDevice {
                port,
                vendor: read(&path, "idVendor"),
                product: read(&path, "idProduct"),
                manufacturer: read(&path, "manufacturer"),
                name: read(&path, "product"),
                serial: read(&path, "serial"),
                node: format!("{node} {access}"),
                path,
                adb,
                interfaces,
            })
        })
        .collect();
    devices.sort_by(|a, b| a.port.cmp(&b.port));
    devices
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

/// Whether a `/proc/net/tcp` table has a socket in LISTEN state (`0A`) on
/// `port`, any local address.
pub fn tcp_table_listens(table: &str, port: u16) -> bool {
    let wanted = format!("{port:04X}");
    table.lines().skip(1).any(|line| {
        let mut fields = line.split_whitespace();
        let local = fields.nth(1).unwrap_or_default();
        let state = fields.nth(1).unwrap_or_default();
        state == "0A" && local.rsplit(':').next() == Some(wanted.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_listening_port_in_proc_net_tcp() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   \
            0: 0100007F:13AD 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 1 1 0 100 0 0 10 0\n   \
            1: 0100007F:13AE 0100007F:9C40 01 00000000:00000000 00:00000000 00000000   966        0 2 1 0 20 4 30 10 -1\n";
        assert!(tcp_table_listens(table, 5037));
        assert!(
            !tcp_table_listens(table, 5038),
            "established, not listening"
        );
        assert!(!tcp_table_listens(table, 22));
    }
}
