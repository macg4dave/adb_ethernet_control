//! `boot`: the once-per-boot check and bounded recovery run by `adb-wan.service`.

use crate::adb::usb_state::UsbEthernet;
use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::log::quote;
use crate::net::WanHealth;
use crate::net::network_manager::NmState;
use crate::record::Record;
use crate::recovery::cycle::{
    ToggleReport, check_wan, cycle, enable, usb_now, wait_nm_finished, wait_phone, wait_valid_usb,
    wait_wan,
};
use crate::recovery::decision::{Decision, decide};
use crate::units::readiness::check_ready;

/// Facts for the final `result=...` line and the record.
#[derive(Debug, Clone)]
struct Summary {
    action: &'static str,
    changed: bool,
    usb: &'static str,
    wan: &'static str,
}

/// Runs the boot flow and always ends with one summary line.
pub fn run(ctx: &Ctx<'_>, record: Option<&Record>) -> Result<Exit> {
    let started = ctx.clock.now();
    let mut summary = Summary {
        action: "none",
        changed: false,
        usb: "not-read",
        wan: "unknown",
    };
    let outcome = flow(ctx, &mut summary);
    ctx.adb.stop_server();
    let result = match &outcome {
        Ok(Exit::Success) if summary.changed => "passed",
        Ok(Exit::Success) => "healthy-no-action",
        Ok(Exit::Failed) => "failed",
        Err(_) => "error",
    };
    if let Err(e) = &outcome {
        ctx.log
            .error(format_args!("boot error={}", quote(&e.to_string())));
    }
    let elapsed = (ctx.clock.now() - started).as_secs();
    let line = format!(
        "result={result} action={} changed_by_us={} usb={} wan={} elapsed_s={elapsed}",
        summary.action,
        if summary.changed { "yes" } else { "no" },
        summary.usb,
        summary.wan,
    );
    if result == "passed" || result == "healthy-no-action" {
        ctx.log.info(&line);
    } else {
        ctx.log.error(&line);
    }
    if let Some(record) = record {
        let exit_code = match &outcome {
            Ok(exit) => exit.code(),
            Err(_) => 1,
        };
        let status = if exit_code == 0 { "passed" } else { "failed" };
        let written = record.write_result(&[
            ("status", status.into()),
            ("summary", line),
            ("exit_code", exit_code.to_string()),
        ]);
        if let Err(e) = written {
            ctx.log.warn(format_args!(
                "record not written error={}",
                quote(&e.to_string())
            ));
        }
        ctx.log
            .info(format_args!("record path={}", record.dir().display()));
    }
    outcome
}

#[allow(
    clippy::too_many_lines,
    reason = "a linear list of logged stages; splitting it hides their order"
)]
fn flow(ctx: &Ctx<'_>, summary: &mut Summary) -> Result<Exit> {
    let config = ctx.config;

    // systemd started us after the readiness units (After= drop-in).
    check_ready(ctx);

    let start = check_wan(ctx, "start");
    if start.is_up() {
        summary.wan = "up";
        ctx.log
            .info("WAN healthy; no changes needed and ADB not started");
        return Ok(Exit::Success);
    }
    summary.wan = "down";
    ctx.log.warn(format_args!(
        "WAN down at start (\"{start}\"); asking the phone what state it is in"
    ));

    ctx.usb.check(ctx, "start");
    let stage = ctx.log.stage("adb-server");
    ctx.adb.start_server()?;
    stage.finish("ok");

    let stage = ctx.log.stage("phone");
    wait_phone(ctx)?;
    stage.finish("ok");

    let stage = ctx.log.stage("usb-state");
    let (transport, usb) = wait_valid_usb(ctx)?;
    summary.usb = usb.ethernet.word();
    stage.finish(usb.ethernet.word());

    let health = judge_wan(ctx, &usb.ethernet);
    if health.is_up() {
        summary.wan = "up";
        ctx.log
            .info("WAN recovered before any USB change; no changes needed");
        return Ok(Exit::Success);
    }

    let acted = ctx.markers.acted_this_boot();
    if let Some(action) = &acted {
        ctx.log
            .warn(format_args!("already acted this boot action={action}"));
    }
    let decision = decide(
        &usb.ethernet,
        config.phone.usb_function,
        config.recovery.usb_cycle_enabled,
        acted.is_some(),
    );
    ctx.log.info(format_args!(
        "decision {decision} usb_raw={} wan=\"{health}\"",
        quote(&usb.raw)
    ));
    summary.action = decision.action_word();

    let report: Option<ToggleReport> = match decision {
        Decision::Enable(function) => {
            ctx.markers.mark_acted("enable")?;
            summary.changed = true;
            let stage = ctx.log.stage("enable");
            let report = enable(ctx, transport, function);
            stage.finish(if report.on_ok() {
                "on-observed"
            } else {
                "on-not-observed"
            });
            Some(report)
        }
        Decision::Cycle(function) => {
            ctx.markers.mark_acted("cycle")?;
            summary.changed = true;
            let stage = ctx.log.stage("cycle");
            let report = cycle(ctx, transport, function)?;
            stage.finish(if report.on_ok() {
                "on-observed"
            } else {
                "on-not-observed"
            });
            Some(report)
        }
        Decision::NoAction(reason) => {
            ctx.log
                .warn(format_args!("no recovery action reason={reason:?}"));
            None
        }
    };
    // Without an action the WAN state judged above is the result.
    let last = match &report {
        None => health,
        Some(report) => {
            ctx.log.info(format_args!(
                "action report off_observed={:?} off_ms={:?} on_observed={:?} on_ms={:?}",
                report.off_observed, report.off_ms, report.on_observed, report.on_ms
            ));
            let stage = ctx.log.stage("wan-after-action");
            let timeline = wait_wan(ctx, "wan-after-action");
            ctx.log.info(format_args!(
                "wan timeline interface_ms={:?} address_ms={:?} up_ms={:?} last=\"{}\"",
                timeline.interface_ms, timeline.address_ms, timeline.up_ms, timeline.last
            ));
            stage.finish(if timeline.last.is_up() { "up" } else { "down" });
            timeline.last
        }
    };
    let usb = usb_now(ctx);
    summary.usb = usb.word();
    summary.wan = if last.is_up() { "up" } else { "down" };

    if last.is_up() {
        Ok(Exit::Success)
    } else {
        let detail = match &usb {
            UsbEthernet::Unknown(why) => format!("unknown ({why})"),
            other => other.to_string(),
        };
        ctx.log.error(format_args!(
            "USB Ethernet {detail} but WAN unavailable (\"{last}\"); stopping without further actions"
        ));
        Ok(Exit::Failed)
    }
}

/// Judges the WAN once the phone's state is known. With USB Ethernet on,
/// NetworkManager may still be bringing the interface up after boot, so
/// wait for it to finish (connected or failed) instead of guessing a delay.
/// With it off, nothing will bring the WAN up by itself: judge now.
fn judge_wan(ctx: &Ctx<'_>, usb: &UsbEthernet) -> WanHealth {
    if !matches!(usb, UsbEthernet::On(_)) {
        return check_wan(ctx, "pre-action");
    }
    let stage = ctx.log.stage("networkmanager");
    let nm = wait_nm_finished(ctx);
    stage.finish(nm);
    if nm == NmState::Connected {
        // Connected: give the first packets a chance before calling it down.
        wait_wan(ctx, "wan-connected").last
    } else {
        check_wan(ctx, "pre-action")
    }
}
