//! `status`: read-only report of host interface, WAN health, ADB devices and phone USB state.

use std::io::Write;
use std::time::Duration;

use crate::adb::device::Selection;
use crate::adb::{self, getprop};
use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::net::network_manager::device_show_args;
use crate::net::wan_health;
use crate::process::{Cmd, run as run_cmd};

/// Prints the report to stdout. Exit 2 when any part could not be read.
#[allow(
    clippy::too_many_lines,
    reason = "a linear report; splitting it hides its order"
)]
pub fn run(ctx: &Ctx<'_>) -> Result<Exit> {
    let config = ctx.config;
    let iface = &config.wan.interface;
    let mut out = std::io::stdout().lock();
    let mut partial = false;

    let _ = writeln!(out, "== Host interface {iface} ==");
    match ctx.host.interface(iface) {
        Ok(status) => {
            let _ = writeln!(
                out,
                "present={} driver={} ipv4={}",
                status.present,
                status.driver.as_deref().unwrap_or("-"),
                status.ipv4.as_deref().unwrap_or("-")
            );
        }
        Err(e) => {
            partial = true;
            let _ = writeln!(out, "unavailable: {e}");
        }
    }
    match wan_health(ctx.host, iface) {
        Ok(health) => {
            let _ = writeln!(out, "wan={health}");
        }
        Err(e) => {
            partial = true;
            let _ = writeln!(out, "wan check unavailable: {e}");
        }
    }
    let nm = Cmd::new(&config.paths.nmcli, Duration::from_secs(10)).args(device_show_args(iface));
    match run_cmd(&nm) {
        Ok(output) if output.success() => {
            let _ = writeln!(out, "{}", output.stdout.trim());
        }
        Ok(output) => {
            let _ = writeln!(out, "nmcli: {}", output.summary());
        }
        Err(e) => {
            let _ = writeln!(out, "nmcli unavailable: {e}");
        }
    }

    let _ = writeln!(out, "\n== Phone ({}) ==", config.phone.serial.redacted());
    if let Err(e) = ctx.adb.start_server() {
        let _ = writeln!(out, "ADB server: {e}");
        return Ok(Exit::Failed);
    }
    match ctx.adb.devices() {
        Ok(listing) => {
            let _ = writeln!(out, "adb devices -l:\n{}", listing.trim_end());
        }
        Err(e) => {
            let _ = writeln!(out, "adb devices failed: {e}");
            ctx.adb.stop_server();
            return Ok(Exit::Failed);
        }
    }
    let transport = match adb::select_phone(ctx.adb, &config.phone.serial)? {
        Selection::Selected(transport) => transport,
        other => {
            let _ = writeln!(out, "phone: {}", other.problem().unwrap_or_default());
            if ctx.host.usb_serial_present(&config.phone.serial) {
                let _ = writeln!(
                    out,
                    "phone is attached on USB: check for another ADB server holding it, the udev rule, or authorization"
                );
            }
            ctx.adb.stop_server();
            return Ok(Exit::Failed);
        }
    };
    let _ = writeln!(out, "transport={transport}");
    for (label, prop) in [
        ("model", "ro.product.model"),
        ("android", "ro.build.version.release"),
        ("sys.usb.config", "sys.usb.config"),
    ] {
        match getprop(ctx.adb, transport, prop) {
            Ok(value) => {
                let _ = writeln!(out, "{label}={value}");
            }
            Err(e) => {
                partial = true;
                let _ = writeln!(out, "{label} unavailable: {e}");
            }
        }
    }
    match adb::usb_state(ctx.adb, transport) {
        Ok(state) => {
            let _ = writeln!(
                out,
                "sys.usb.state={} usb_ethernet={}",
                state.raw, state.ethernet
            );
        }
        Err(e) => {
            partial = true;
            let _ = writeln!(out, "sys.usb.state unavailable: {e}");
        }
    }
    ctx.adb.stop_server();
    Ok(if partial { Exit::Failed } else { Exit::Success })
}
