//! `boot`: the once-per-boot check and bounded recovery run by `adb-wan.service`.

use crate::adb::usb_state::UsbEthernet;
use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::log::quote;
use crate::record::Record;
use crate::recovery::cycle::{
    ToggleReport, check_wan, cycle, enable, final_checks, usb_now, wait_phone, wait_valid_usb,
    wait_wan,
};
use crate::recovery::decision::{Decision, decide};
use crate::units::readiness::wait_ready;

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

    let stage = ctx.log.stage("readiness");
    wait_ready(ctx)?;
    stage.finish("ok");

    let stage = ctx.log.stage("initial-wan");
    let initial = wait_wan(
        ctx,
        "initial-wan",
        config.timing.initial_wan_timeout_secs.get(),
    );
    if initial.last.is_up() {
        stage.finish("up");
        summary.wan = "up";
        ctx.log
            .info("WAN healthy; no changes needed and ADB not started");
        return Ok(Exit::Success);
    }
    stage.finish("down");
    summary.wan = "down";
    ctx.log.warn(format_args!(
        "WAN unavailable after startup last=\"{}\"; inspecting the phone",
        initial.last
    ));

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

    // WAN may have come back while ADB was starting.
    let health = check_wan(ctx, "pre-action");
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
    if let Some(report) = &report {
        ctx.log.info(format_args!(
            "action report off_observed={:?} off_ms={:?} on_observed={:?} on_ms={:?}",
            report.off_observed, report.off_ms, report.on_observed, report.on_ms
        ));
        let stage = ctx.log.stage("wan-after-action");
        let timeline = wait_wan(
            ctx,
            "wan-after-action",
            config.timing.wan_timeout_secs.get(),
        );
        ctx.log.info(format_args!(
            "wan timeline interface_ms={:?} address_ms={:?} up_ms={:?} last=\"{}\"",
            timeline.interface_ms, timeline.address_ms, timeline.up_ms, timeline.last
        ));
        stage.finish(if timeline.last.is_up() { "up" } else { "down" });
    }

    let stage = ctx.log.stage("final-checks");
    let last = final_checks(ctx);
    let usb = usb_now(ctx);
    summary.usb = usb.word();
    summary.wan = if last.is_up() { "up" } else { "down" };
    stage.finish(summary.wan);

    if last.is_up() {
        Ok(Exit::Success)
    } else {
        let detail = match &usb {
            UsbEthernet::Unknown(why) => format!("unknown ({why})"),
            other => other.to_string(),
        };
        ctx.log.error(format_args!(
            "USB Ethernet {detail} but WAN unavailable after final check (\"{last}\"); stopping without further actions"
        ));
        Ok(Exit::Failed)
    }
}
