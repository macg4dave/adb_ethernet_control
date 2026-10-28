//! `test-usb-enable`: with USB Ethernet switched off on the phone, wait for
//! the phone to offer ADB (like `adb wait-for-device`), then run
//! `svc usb setFunctions rndis` once and record what happens.
//!
//! Unlike `boot`, it ignores the once-per-boot guard, so it can be repeated.

use crate::adb::usb_state::UsbEthernet;
use crate::adb::{self, getprop};
use crate::commands::usb_toggle::{supervised, write_record};
use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::limits;
use crate::log::quote;
use crate::record::Record;
use crate::recovery::cycle::{enable, wait_phone, wait_valid_usb, wait_wan};
use crate::wait::wait_until;

/// The test inside the transient unit.
pub fn run(ctx: &Ctx<'_>, record: Option<&Record>) -> Result<Exit> {
    supervised(ctx, record, "test-usb-enable", test)
}

/// Waits for the event "tethering switched off": the phone reports USB
/// Ethernet off, or drops off ADB (this phone turns ADB off with it).
fn wait_for_user_to_switch_off(ctx: &Ctx<'_>) {
    ctx.log
        .warn("USB Ethernet is on: switch USB tethering OFF on the phone now");
    let _ = wait_until(
        ctx.clock,
        ctx.log,
        "tethering-off",
        limits::WAIT,
        limits::POLL,
        || {
            ctx.usb.check(ctx, "tethering-off");
            match adb::observe(ctx.adb, &ctx.config.phone.serial) {
                Ok((_, state)) if state.ethernet == UsbEthernet::Off => Ok(()),
                Ok((_, state)) => Err(format!("still {}", state.ethernet)),
                // Gone from ADB: tethering went off and took ADB with it.
                Err(_) => Ok(()),
            }
        },
    );
}

/// The test body; public so the flow tests can drive it with fakes.
pub fn test(ctx: &Ctx<'_>, record: Option<&Record>) -> Result<Exit> {
    let config = ctx.config;
    let function = config.phone.usb_function;

    let stage = ctx.log.stage("phone");
    ctx.usb.check(ctx, "start");
    ctx.adb.start_server()?;
    ctx.log.info(
        "waiting for the phone to offer ADB (like `adb wait-for-device`); if it never does, the \
         log says what the phone offers instead",
    );
    wait_phone(ctx)?;
    let (_, usb) = wait_valid_usb(ctx)?;
    if usb.ethernet != UsbEthernet::Off {
        wait_for_user_to_switch_off(ctx);
        wait_phone(ctx)?;
    }
    let (transport, usb) = wait_valid_usb(ctx)?;
    stage.finish(usb.ethernet.word());

    let prop =
        |name| getprop(ctx.adb, transport, name).unwrap_or_else(|e| format!("unavailable: {e}"));
    let model = prop("ro.product.model");
    let android = prop("ro.build.version.release");
    let usb_config = prop("sys.usb.config");
    ctx.log.info(format_args!(
        "before model={} android={} sys.usb.state={} sys.usb.config={} transport={transport}",
        quote(&model),
        quote(&android),
        quote(&usb.raw),
        quote(&usb_config)
    ));
    let mut fields: Vec<(&str, String)> = vec![
        ("before_usb_state", usb.raw.clone()),
        ("before_usb_config", usb_config),
        ("function", function.to_string()),
        ("model", model),
        ("android", android),
    ];
    if usb.ethernet != UsbEthernet::Off {
        ctx.log.warn(format_args!(
            "USB Ethernet is still {}; tethering was not switched off on the phone",
            usb.ethernet
        ));
        fields.insert(0, ("status", "skipped".into()));
        write_record(ctx, record, &fields);
        return Ok(Exit::Failed);
    }

    let stage = ctx.log.stage("enable");
    let report = enable(ctx, transport, function);
    stage.finish(if report.on_ok() {
        "on-observed"
    } else {
        "on-not-observed"
    });

    let stage = ctx.log.stage("wan-after-enable");
    let timeline = wait_wan(ctx, "wan-after-enable");
    stage.finish(if timeline.last.is_up() { "up" } else { "down" });

    let passed = report.on_ok() && timeline.last.is_up();
    let ms = |v: Option<u128>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
    fields.splice(
        0..0,
        [
            ("status", if passed { "passed" } else { "failed" }.into()),
            (
                "on_observed",
                report.on_observed.clone().unwrap_or_else(|| "no".into()),
            ),
            ("on_ms", ms(report.on_ms)),
            ("on_command", report.on_command.clone().unwrap_or_default()),
            ("interface_ms", ms(timeline.interface_ms)),
            ("address_ms", ms(timeline.address_ms)),
            ("wan_up_ms", ms(timeline.up_ms)),
            ("final_wan", timeline.last.to_string()),
        ],
    );
    write_record(ctx, record, &fields);
    if passed {
        ctx.log
            .info("test result=pass: phone offered ADB with USB Ethernet off, rndis was enabled and WAN works");
        Ok(Exit::Success)
    } else {
        ctx.log
            .error("test result=fail: see on_observed and final_wan in the record");
        Ok(Exit::Failed)
    }
}
