//! The recovery steps shared by `boot`, `restore` and the manual test: waits
//! for the phone, the single enable or off/on cycle, and the WAN checks.

use std::time::Duration;

use crate::adb::device::{Selection, TransportId};
use crate::adb::usb_state::{NetFunction, UsbEthernet, UsbState};
use crate::adb::{self, set_functions};
use crate::context::Ctx;
use crate::error::{Error, Result};
use crate::log::quote;
use crate::net::{WanHealth, wan_health};
use crate::wait::{Waited, wait_until};

/// Waits for the configured phone to be listed, authorized and on USB.
pub fn wait_phone(ctx: &Ctx<'_>) -> Result<TransportId> {
    let serial = &ctx.config.phone.serial;
    let waited = wait_until(
        ctx.clock,
        ctx.log,
        "phone",
        ctx.config.timing.phone_timeout_secs.get(),
        ctx.config.timing.poll_interval_secs.get(),
        || match adb::select_phone(ctx.adb, serial) {
            Ok(Selection::Selected(transport)) => Ok(transport),
            Ok(other) => Err(other.problem().unwrap_or_default()),
            Err(e) => Err(e.to_string()),
        },
    );
    match waited {
        Waited::Met { value, .. } => {
            ctx.log
                .info(format_args!("phone selected transport={value}"));
            Ok(value)
        }
        Waited::TimedOut { last, .. } => {
            if ctx.host.usb_serial_present(serial) {
                ctx.log.warn(
                    "phone is attached on USB but our ADB server cannot use it: another ADB \
                     server (e.g. a desktop adb on port 5037) may hold it, the udev rule may \
                     be missing, or the phone has not authorized this host's key",
                );
            } else {
                ctx.log
                    .warn("no USB device with the phone's serial is attached");
            }
            Err(Error::Control(format!("phone not available: {last}")))
        }
    }
}

/// Re-selects the phone and reads its USB state until `accept` holds.
pub fn wait_usb(
    ctx: &Ctx<'_>,
    what: &str,
    timeout: Duration,
    accept: impl Fn(&UsbEthernet) -> bool,
) -> Waited<(TransportId, UsbState)> {
    wait_until(
        ctx.clock,
        ctx.log,
        what,
        timeout,
        ctx.config.timing.poll_interval_secs.get(),
        || {
            let (transport, state) = adb::observe(ctx.adb, &ctx.config.phone.serial)?;
            if accept(&state.ethernet) {
                ctx.log.info(format_args!(
                    "usb state read raw={} parsed={} transport={transport}",
                    quote(&state.raw),
                    state.ethernet
                ));
                Ok((transport, state))
            } else {
                Err(format!("raw={} parsed={}", state.raw, state.ethernet))
            }
        },
    )
}

/// Time allowed for the phone to re-enumerate and report a changed state.
fn usb_change_timeout(ctx: &Ctx<'_>) -> Duration {
    ctx.config.timing.phone_timeout_secs.get() + ctx.config.timing.usb_state_timeout_secs.get()
}

/// Waits for a trustworthy (not unknown) USB state.
pub fn wait_valid_usb(ctx: &Ctx<'_>) -> Result<(TransportId, UsbState)> {
    match wait_usb(
        ctx,
        "usb-state",
        ctx.config.timing.usb_state_timeout_secs.get(),
        |e| !matches!(e, UsbEthernet::Unknown(_)),
    ) {
        Waited::Met { value, .. } => Ok(value),
        Waited::TimedOut { last, .. } => Err(Error::Control(format!(
            "cannot establish USB Ethernet state ({last}); no USB changes made"
        ))),
    }
}

/// What happened during an enable or cycle; also the hardware test record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToggleReport {
    /// Result of the off command, if sent.
    pub off_command: Option<String>,
    /// State observed after off, if it was observed as off.
    pub off_observed: Option<String>,
    /// Milliseconds from the off command until off was observed (ADB back).
    pub off_ms: Option<u128>,
    /// Result of the on command, if sent.
    pub on_command: Option<String>,
    /// State observed after on, if it was observed as on.
    pub on_observed: Option<String>,
    /// Milliseconds from the on command until on was observed (ADB back).
    pub on_ms: Option<u128>,
}

impl ToggleReport {
    /// Whether USB Ethernet was observed on after the action.
    pub fn on_ok(&self) -> bool {
        self.on_observed.is_some()
    }
}

fn send(ctx: &Ctx<'_>, transport: TransportId, functions: &str) -> String {
    ctx.log.info(format_args!(
        "action setFunctions functions={functions} transport={transport}"
    ));
    let summary = match set_functions(ctx.adb, transport, functions) {
        Ok(output) => output.summary(),
        Err(e) => format!("error={}", quote(&e.to_string())),
    };
    // Responses are often lost while USB re-enumerates; the observed state decides.
    ctx.log.info(format_args!("action response {summary}"));
    summary
}

fn turn_on(
    ctx: &Ctx<'_>,
    transport: TransportId,
    function: NetFunction,
    report: &mut ToggleReport,
) {
    let sent = ctx.clock.now();
    report.on_command = Some(send(ctx, transport, function.as_str()));
    if let Waited::Met {
        value: (_, state), ..
    } = wait_usb(ctx, "usb-on", usb_change_timeout(ctx), |e| {
        matches!(e, UsbEthernet::On(_))
    }) {
        report.on_observed = Some(state.raw);
        report.on_ms = Some((ctx.clock.now() - sent).as_millis());
    }
}

/// Requests `function` once and waits to observe USB Ethernet on.
pub fn enable(ctx: &Ctx<'_>, transport: TransportId, function: NetFunction) -> ToggleReport {
    let mut report = ToggleReport::default();
    turn_on(ctx, transport, function, &mut report);
    report
}

/// One off/on cycle. The restore marker is armed first (the cycle is
/// refused if that fails) and cleared only once on is observed.
pub fn cycle(ctx: &Ctx<'_>, transport: TransportId, function: NetFunction) -> Result<ToggleReport> {
    ctx.markers.set_restore_pending(function)?;
    ctx.log
        .info(format_args!("restore armed function={function}"));
    let mut report = ToggleReport::default();
    let sent = ctx.clock.now();
    report.off_command = Some(send(ctx, transport, &ctx.config.recovery.off_function));
    let after_off = match wait_usb(ctx, "usb-off", usb_change_timeout(ctx), |e| {
        *e == UsbEthernet::Off
    }) {
        Waited::Met {
            value: (transport, state),
            ..
        } => {
            report.off_observed = Some(state.raw);
            report.off_ms = Some((ctx.clock.now() - sent).as_millis());
            Some(transport)
        }
        Waited::TimedOut { .. } => {
            ctx.log.warn("off state not observed; requesting on anyway");
            match adb::select_phone(ctx.adb, &ctx.config.phone.serial) {
                Ok(Selection::Selected(transport)) => Some(transport),
                _ => None,
            }
        }
    };
    let Some(transport) = after_off else {
        ctx.log
            .error("ADB did not return after off; the ExecStopPost restore stays armed");
        return Ok(report);
    };
    turn_on(ctx, transport, function, &mut report);
    if report.on_ok() {
        ctx.markers.clear_restore_pending()?;
        ctx.log.info("restore disarmed: USB Ethernet observed on");
    } else {
        ctx.log
            .error("USB Ethernet not observed on; the ExecStopPost restore stays armed");
    }
    Ok(report)
}

/// When each part of the WAN came up during a wait, in ms from its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WanTimeline {
    /// Interface first present.
    pub interface_ms: Option<u128>,
    /// Global IPv4 address first present.
    pub address_ms: Option<u128>,
    /// Probe first passed.
    pub up_ms: Option<u128>,
    /// Last health observed.
    pub last: WanHealth,
}

/// Waits for WAN to work, noting when the interface, address and probe
/// each first succeeded.
pub fn wait_wan(ctx: &Ctx<'_>, what: &str, timeout: Duration) -> WanTimeline {
    let start = ctx.clock.now();
    let mut timeline = WanTimeline {
        interface_ms: None,
        address_ms: None,
        up_ms: None,
        last: WanHealth::NoInterface,
    };
    let interface = &ctx.config.wan.interface;
    let _ = wait_until(
        ctx.clock,
        ctx.log,
        what,
        timeout,
        ctx.config.timing.poll_interval_secs.get(),
        || {
            let health = wan_health(ctx.host, interface).map_err(|e| e.to_string())?;
            let at = Some((ctx.clock.now() - start).as_millis());
            if !matches!(health, WanHealth::NoInterface) && timeline.interface_ms.is_none() {
                timeline.interface_ms = at;
            }
            if matches!(health, WanHealth::Up { .. } | WanHealth::ProbeFailed { .. })
                && timeline.address_ms.is_none()
            {
                timeline.address_ms = at;
            }
            timeline.last = health.clone();
            if health.is_up() {
                timeline.up_ms = at;
                Ok(())
            } else {
                Err(health.to_string())
            }
        },
    );
    timeline
}

/// One WAN check, logged. Errors count as down.
pub fn check_wan(ctx: &Ctx<'_>, label: &str) -> WanHealth {
    let health = wan_health(ctx.host, &ctx.config.wan.interface).unwrap_or_else(|e| {
        ctx.log
            .warn(format_args!("wan check error={}", quote(&e.to_string())));
        WanHealth::NoInterface
    });
    ctx.log
        .info(format_args!("wan check name={label} {health}"));
    health
}

/// Checks WAN, waits the confirm interval, checks again; the second decides.
pub fn final_checks(ctx: &Ctx<'_>) -> WanHealth {
    check_wan(ctx, "final-1");
    let interval = ctx.config.timing.confirm_interval_secs.get();
    ctx.log
        .info(format_args!("confirm interval secs={}", interval.as_secs()));
    ctx.clock.sleep(interval);
    check_wan(ctx, "final-2")
}

/// One USB state read without waiting, for final summaries.
pub fn usb_now(ctx: &Ctx<'_>) -> UsbEthernet {
    match adb::observe(ctx.adb, &ctx.config.phone.serial) {
        Ok((_, state)) => state.ethernet,
        Err(observation) => UsbEthernet::Unknown(observation),
    }
}
