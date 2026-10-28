//! Hardware tests: launched under the system manager on request, and the
//! recorded `test-usb-toggle` (one USB off/on).

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::account;
use crate::adb::getprop;
use crate::adb::usb_state::UsbEthernet;
use crate::config::Config;
use crate::context::Ctx;
use crate::error::{Error, Exit, Result};
use crate::log::{Log, quote};
use crate::process::{Cmd, run as run_cmd};
use crate::record::Record;
use crate::recovery::cycle::{check_wan, cycle, wait_phone, wait_valid_usb, wait_wan};
use crate::units::service::{BINARY, TEST_UNIT, UNIT_NAME, test_run_args};

const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";
/// Longer than the shared `TimeoutStartSec=` plus `TimeoutStopSec=`.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(25 * 60);

/// Run from a terminal with sudo: start the test as a transient service so
/// the system manager owns it, wait for it, then print its record.
pub fn launch(config: &Config, log: &Log, subcommand: &str, record_prefix: &str) -> Result<Exit> {
    if !account::is_root() {
        return Err(Error::Control(format!(
            "run with sudo: sudo adb-wan {subcommand}"
        )));
    }
    if !Path::new(BINARY).exists() {
        return Err(Error::Control(format!(
            "{BINARY} missing; run `sudo adb-wan install` first"
        )));
    }
    let state = |unit: &str| {
        run_cmd(
            &Cmd::new(&config.paths.systemctl, Duration::from_secs(5)).args(["is-active", unit]),
        )
        .map(|o| o.stdout.trim().to_owned())
        .unwrap_or_default()
    };
    if state(UNIT_NAME) == "activating" {
        return Err(Error::Control(format!(
            "{UNIT_NAME} is running; wait for it to finish"
        )));
    }
    if matches!(
        state(&format!("{TEST_UNIT}.service")).as_str(),
        "active" | "activating"
    ) {
        return Err(Error::Control("a hardware test is already running".into()));
    }
    log.info(format_args!(
        "launching {TEST_UNIT}.service under the system manager; it continues if this terminal, the WAN or any chat session is lost"
    ));
    log.info("watch it live in another terminal: journalctl -f -t adb-wan");
    let output = run_cmd(&Cmd::new(SYSTEMD_RUN, LAUNCH_TIMEOUT).args(test_run_args(subcommand)))?;
    log.info(format_args!("systemd-run finished {}", output.summary()));
    if let Some(dir) = newest_record(&config.paths.log_dir, record_prefix) {
        let result = std::fs::read_to_string(dir.join("result")).unwrap_or_default();
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "\nRecord: {}\n{result}", dir.display());
    }
    Ok(if output.success() {
        Exit::Success
    } else {
        Exit::Failed
    })
}

fn newest_record(log_dir: &Path, prefix: &str) -> Option<std::path::PathBuf> {
    let mut dirs: Vec<_> = std::fs::read_dir(log_dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.pop()
}

/// The toggle test itself, inside the transient unit.
pub fn run(ctx: &Ctx<'_>, record: Option<&Record>) -> Result<Exit> {
    supervised(ctx, record, "test-usb-toggle", test)
}

/// Runs a hardware test body inside the transient unit: refuses to run
/// outside systemd, always stops ADB, and records an error as a failure.
pub fn supervised(
    ctx: &Ctx<'_>,
    record: Option<&Record>,
    subcommand: &str,
    body: fn(&Ctx<'_>, Option<&Record>) -> Result<Exit>,
) -> Result<Exit> {
    // INVOCATION_ID is set by systemd for every unit; not a setting.
    if std::env::var_os("INVOCATION_ID").is_none() {
        return Err(Error::Control(format!(
            "must run under the system manager; use: sudo adb-wan {subcommand}"
        )));
    }
    let outcome = body(ctx, record);
    ctx.adb.stop_server();
    if let Err(e) = &outcome {
        write_record(
            ctx,
            record,
            &[("status", "failed".into()), ("error", e.to_string())],
        );
    }
    outcome
}

/// Writes a test record and logs where it is.
pub fn write_record(ctx: &Ctx<'_>, record: Option<&Record>, fields: &[(&str, String)]) {
    let line = fields
        .iter()
        .map(|(k, v)| format!("{k}={}", quote(v)))
        .collect::<Vec<_>>()
        .join(" ");
    ctx.log.info(format_args!("test {line}"));
    if let Some(record) = record {
        if let Err(e) = record.write_result(fields) {
            ctx.log.warn(format_args!(
                "record not written error={}",
                quote(&e.to_string())
            ));
        }
        ctx.log
            .info(format_args!("record path={}", record.dir().display()));
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "a linear list of logged test stages; splitting it hides their order"
)]
fn test(ctx: &Ctx<'_>, record: Option<&Record>) -> Result<Exit> {
    let config = ctx.config;
    let stage = ctx.log.stage("baseline");
    ctx.usb.check(ctx, "baseline");
    ctx.adb.start_server()?;
    wait_phone(ctx)?;
    let (transport, usb) = wait_valid_usb(ctx)?;
    let UsbEthernet::On(function) = usb.ethernet else {
        return Err(Error::Control(format!(
            "test needs USB Ethernet on (found {}); no changes made",
            usb.ethernet
        )));
    };
    let baseline = check_wan(ctx, "baseline");
    if !baseline.is_up() {
        return Err(Error::Control(format!(
            "test needs a healthy WAN baseline (found \"{baseline}\"); no changes made"
        )));
    }
    let prop =
        |name| getprop(ctx.adb, transport, name).unwrap_or_else(|e| format!("unavailable: {e}"));
    let model = prop("ro.product.model");
    let android = prop("ro.build.version.release");
    let usb_config = prop("sys.usb.config");
    let driver = ctx
        .host
        .interface(&config.wan.interface)
        .ok()
        .and_then(|s| s.driver)
        .unwrap_or_else(|| "-".into());
    ctx.log.info(format_args!(
        "baseline model={} android={} sys.usb.state={} sys.usb.config={} driver={driver} transport={transport}",
        quote(&model),
        quote(&android),
        quote(&usb.raw),
        quote(&usb_config)
    ));
    stage.finish("ok");

    let stage = ctx.log.stage("toggle");
    let report = cycle(ctx, transport, function)?;
    stage.finish(if report.on_ok() {
        "on-observed"
    } else {
        "on-not-observed"
    });

    let stage = ctx.log.stage("wan-after-toggle");
    let timeline = wait_wan(ctx, "wan-after-toggle");
    stage.finish(if timeline.last.is_up() { "up" } else { "down" });
    let last = timeline.last.clone();

    let passed = report.off_observed.is_some() && report.on_observed.is_some();
    let ms = |v: Option<u128>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
    let fields: Vec<(&str, String)> = vec![
        ("status", if passed { "passed" } else { "failed" }.into()),
        (
            "off_observed",
            report.off_observed.clone().unwrap_or_else(|| "no".into()),
        ),
        ("off_ms", ms(report.off_ms)),
        (
            "off_command",
            report.off_command.clone().unwrap_or_default(),
        ),
        (
            "on_observed",
            report.on_observed.clone().unwrap_or_else(|| "no".into()),
        ),
        ("on_ms", ms(report.on_ms)),
        ("on_command", report.on_command.clone().unwrap_or_default()),
        ("interface_ms", ms(timeline.interface_ms)),
        ("address_ms", ms(timeline.address_ms)),
        ("wan_up_ms", ms(timeline.up_ms)),
        ("final_wan", last.to_string()),
        (
            "restore_pending",
            ctx.markers.restore_pending().is_some().to_string(),
        ),
        ("function", function.to_string()),
        ("off_function", config.recovery.off_function.clone()),
        ("baseline_usb_state", usb.raw.clone()),
        ("baseline_usb_config", usb_config),
        ("model", model),
        ("android", android),
        ("driver", driver),
    ];
    write_record(ctx, record, &fields);
    if passed {
        ctx.log
            .info("test result=pass: USB Ethernet observed off and on, ADB returned both times");
        Ok(Exit::Success)
    } else {
        ctx.log.error("test result=fail: see off_observed/on_observed; check `journalctl -t adb-wan` for the ExecStopPost restore");
        Ok(Exit::Failed)
    }
}
