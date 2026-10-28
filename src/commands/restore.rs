//! `restore`: enable-only fallback run by the service's `ExecStopPost=` when a restore is pending.

use crate::adb::usb_state::UsbEthernet;
use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::recovery::cycle::{enable, wait_phone, wait_valid_usb};

/// Re-enables USB Ethernet if a disruptive step did not finish. Never turns
/// anything off.
pub fn run(ctx: &Ctx<'_>) -> Result<Exit> {
    // Set by systemd for ExecStopPost=; logged so the journal shows why the
    // main run ended. These are not settings.
    let env = |key: &str| std::env::var(key).unwrap_or_else(|_| "-".into());
    ctx.log.info(format_args!(
        "restore check service_result={} exit_code={} exit_status={}",
        env("SERVICE_RESULT"),
        env("EXIT_CODE"),
        env("EXIT_STATUS")
    ));
    let Some(function) = ctx.markers.restore_pending() else {
        ctx.log.info("no restore pending; nothing to do");
        return Ok(Exit::Success);
    };
    ctx.log.warn(format_args!(
        "restore pending function={function}: a USB off/on did not finish; re-enabling (enable only)"
    ));
    let outcome = restore(ctx, function);
    ctx.adb.stop_server();
    outcome
}

fn restore(ctx: &Ctx<'_>, function: crate::adb::usb_state::NetFunction) -> Result<Exit> {
    ctx.adb.start_server()?;
    wait_phone(ctx)?;
    let (transport, state) = wait_valid_usb(ctx)?;
    if let UsbEthernet::On(_) = state.ethernet {
        ctx.markers.clear_restore_pending()?;
        ctx.log
            .info("restore result=not-needed: USB Ethernet already on");
        return Ok(Exit::Success);
    }
    let report = enable(ctx, transport, function);
    if report.on_ok() {
        ctx.markers.clear_restore_pending()?;
        ctx.log.info(format_args!(
            "restore result=restored on_ms={:?}",
            report.on_ms
        ));
        Ok(Exit::Success)
    } else {
        ctx.log
            .error("restore result=failed: USB Ethernet not observed on; manual tethering needed");
        Ok(Exit::Failed)
    }
}
