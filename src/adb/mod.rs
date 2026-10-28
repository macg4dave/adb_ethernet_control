//! ADB access through our own loopback server, pinned to one phone.

pub mod device;
pub mod server;
pub mod usb_state;

use crate::error::{Error, Result};
use crate::process::CommandOutput;
use device::{Selection, Serial, TransportId};
use usb_state::UsbState;

/// ADB operations used by the flows. Faked in tests.
pub trait Adb {
    /// Starts our ADB server if it is not running, and waits until it answers.
    fn start_server(&self) -> Result<()>;
    /// Raw `adb devices -l` output.
    fn devices(&self) -> Result<String>;
    /// Runs a fixed command on the phone over one transport.
    fn shell(&self, transport: TransportId, args: &[&str]) -> Result<CommandOutput>;
    /// Stops our ADB server if we started it.
    fn stop_server(&self);
}

/// Looks for the configured phone in the current device list.
pub fn select_phone(adb: &dyn Adb, serial: &Serial) -> Result<Selection> {
    let listing = adb.devices()?;
    Ok(device::select(&device::parse(&listing), serial))
}

/// Reads one Android system property.
pub fn getprop(adb: &dyn Adb, transport: TransportId, name: &str) -> Result<String> {
    let output = adb.shell(transport, &["getprop", name])?;
    if output.success() {
        Ok(output.stdout.trim().to_owned())
    } else {
        Err(Error::Control(format!(
            "getprop {name} failed: {}",
            output.summary()
        )))
    }
}

/// Reads and interprets `sys.usb.state`.
pub fn usb_state(adb: &dyn Adb, transport: TransportId) -> Result<UsbState> {
    getprop(adb, transport, "sys.usb.state").map(|raw| usb_state::parse(&raw))
}

/// Requests a USB function set. `adb` is never included: `svc` keeps ADB
/// enabled itself. The response is often lost during re-enumeration, so
/// callers judge success by the state observed afterwards.
pub fn set_functions(
    adb: &dyn Adb,
    transport: TransportId,
    functions: &str,
) -> Result<CommandOutput> {
    adb.shell(transport, &["svc", "usb", "setFunctions", functions])
}

/// Selects the phone and reads its USB state in one step, for waits.
/// `Err` carries a human-readable observation.
pub fn observe(
    adb: &dyn Adb,
    serial: &Serial,
) -> std::result::Result<(TransportId, UsbState), String> {
    let transport = match select_phone(adb, serial) {
        Ok(Selection::Selected(transport)) => transport,
        Ok(other) => return Err(other.problem().unwrap_or_default()),
        Err(e) => return Err(format!("adb devices failed: {e}")),
    };
    usb_state(adb, transport)
        .map(|state| (transport, state))
        .map_err(|e| format!("transport={transport} {e}"))
}
