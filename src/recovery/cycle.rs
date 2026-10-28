//! The recovery steps shared by `boot`, `restore` and the manual test: waits
//! for the phone, the single enable or off/on cycle, and the WAN checks.

use std::cell::RefCell;

use crate::adb::device::{self, Selection, TransportId};
use crate::adb::usb_state::{NetFunction, UsbEthernet, UsbState};
use crate::adb::{self, set_functions};
use crate::context::Ctx;
use crate::error::{Error, Result};
use crate::limits;
use crate::log::quote;
use crate::net::network_manager::NmState;
use crate::net::{WanHealth, wan_health};
use crate::usb::{self, UsbDevice};
use crate::wait::{Waited, wait_until};

/// Logs the lsusb-style scan whenever it changes, with what disappeared and
/// appeared, so a phone that changes vendor ID, serial or port when USB
/// Ethernet is switched shows up in the journal as a before/after pair.
#[derive(Default)]
pub struct UsbWatch {
    last: RefCell<Option<Vec<UsbDevice>>>,
}

impl UsbWatch {
    /// Scans USB and logs it if it differs from the last scan logged.
    pub fn check(&self, ctx: &Ctx<'_>, at: &str) {
        let now = ctx.host.usb_scan();
        let mut last = self.last.borrow_mut();
        if let Some(before) = last.as_ref() {
            if *before == now {
                return;
            }
            let gone: Vec<String> = before
                .iter()
                .filter(|d| !now.contains(d))
                .map(UsbDevice::describe)
                .collect();
            let new: Vec<String> = now
                .iter()
                .filter(|d| !before.contains(d))
                .map(UsbDevice::describe)
                .collect();
            ctx.log.info(format_args!(
                "usb change at={at} removed=[{}] added=[{}]",
                gone.join(" | "),
                new.join(" | ")
            ));
        }
        ctx.log.info(format_args!(
            "usb scan at={at} devices={}: {}",
            now.len(),
            usb::lsusb(&now)
        ));
        *last = Some(now);
    }
}

/// Waits for the configured phone to be listed, authorized and on USB.
pub fn wait_phone(ctx: &Ctx<'_>) -> Result<TransportId> {
    let serial = &ctx.config.phone.serial;
    let waited = wait_until(
        ctx.clock,
        ctx.log,
        "phone",
        limits::WAIT,
        limits::POLL,
        || {
            ctx.usb.check(ctx, "phone-wait");
            let listing = ctx.adb.devices().map_err(|e| e.to_string())?;
            match device::select(&device::parse(&listing), serial) {
                Selection::Selected(transport) => Ok(transport),
                // Everything needed to tell "no ADB in this USB mode" from
                // "permissions" or "another server" from the journal alone.
                other => Err(format!(
                    "{}; adb_devices=[{}]; phone_usb={}",
                    other.problem().unwrap_or_default(),
                    listing
                        .lines()
                        .filter(|l| !l.trim().is_empty() && !l.starts_with("List of"))
                        .collect::<Vec<_>>()
                        .join(" / "),
                    usb::find_phone(&ctx.host.usb_scan(), serial.as_str())
                        .map_or_else(|| "absent".into(), UsbDevice::describe)
                )),
            }
        },
    );
    match waited {
        Waited::Met { value, .. } => {
            ctx.log
                .info(format_args!("phone selected transport={value}"));
            Ok(value)
        }
        Waited::TimedOut { last, .. } => {
            explain_unreachable(ctx);
            Err(Error::Control(format!("phone not available: {last}")))
        }
    }
}

/// Says in plain words why ADB could not reach the phone, from the kernel's
/// view of USB.
fn explain_unreachable(ctx: &Ctx<'_>) {
    let serial = ctx.config.phone.serial.as_str();
    let scan = ctx.host.usb_scan();
    let others = usb::other_adb_devices(&scan, serial);
    match usb::find_phone(&scan, serial) {
        Some(phone) if !phone.adb => ctx.log.error(format_args!(
            "phone is on USB but offers no ADB interface in this USB mode ({}); it cannot be \
             reached or switched over ADB",
            phone.describe()
        )),
        Some(phone) if ctx.host.tcp_listening(5037) => ctx.log.error(format_args!(
            "phone offers ADB on USB ({}) but another ADB server is listening on \
             127.0.0.1:5037 (a desktop `adb`, started by any `adb` command) and has most \
             likely claimed it; only one server can hold the phone. Stop it with `adb kill-server` \
             as the user who started it",
            phone.describe()
        )),
        Some(phone) => ctx.log.warn(format_args!(
            "phone offers ADB on USB but our ADB server cannot use it ({}): another ADB server \
             (e.g. a desktop adb on port 5037) may hold it, or the phone has not authorized \
             this host's key",
            phone.describe()
        )),
        None if !others.is_empty() => ctx.log.error(format_args!(
            "no USB device with the phone's serial, but ADB-capable device(s) are attached: {}; \
             the phone may report a different serial or vendor ID in this USB mode",
            others
                .iter()
                .map(|d| d.describe())
                .collect::<Vec<_>>()
                .join(" | ")
        )),
        None => ctx
            .log
            .warn("no USB device with the phone's serial is attached"),
    }
}

/// Re-selects the phone and reads its USB state until `accept` holds.
pub fn wait_usb(
    ctx: &Ctx<'_>,
    what: &str,
    accept: impl Fn(&UsbEthernet) -> bool,
) -> Waited<(TransportId, UsbState)> {
    wait_until(ctx.clock, ctx.log, what, limits::WAIT, limits::POLL, || {
        ctx.usb.check(ctx, what);
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
    })
}

/// Waits for a trustworthy (not unknown) USB state.
pub fn wait_valid_usb(ctx: &Ctx<'_>) -> Result<(TransportId, UsbState)> {
    match wait_usb(ctx, "usb-state", |e| !matches!(e, UsbEthernet::Unknown(_))) {
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
    } = wait_usb(ctx, "usb-on", |e| matches!(e, UsbEthernet::On(_)))
    {
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
    let after_off = match wait_usb(ctx, "usb-off", |e| *e == UsbEthernet::Off) {
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
pub fn wait_wan(ctx: &Ctx<'_>, what: &str) -> WanTimeline {
    let start = ctx.clock.now();
    let mut timeline = WanTimeline {
        interface_ms: None,
        address_ms: None,
        up_ms: None,
        last: WanHealth::NoInterface,
    };
    let interface = &ctx.config.wan.interface;
    let _ = wait_until(ctx.clock, ctx.log, what, limits::WAIT, limits::POLL, || {
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
    });
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

/// Waits for the event that NetworkManager has finished with the WAN
/// interface: connected or failed. Returns the last state seen.
pub fn wait_nm_finished(ctx: &Ctx<'_>) -> NmState {
    let interface = &ctx.config.wan.interface;
    let mut last = NmState::Absent;
    let _ = wait_until(
        ctx.clock,
        ctx.log,
        "networkmanager",
        limits::WAIT,
        limits::POLL,
        || {
            last = ctx.host.nm_state(interface).map_err(|e| e.to_string())?;
            if last.finished() {
                Ok(())
            } else {
                Err(format!("state={last}"))
            }
        },
    );
    ctx.log.info(format_args!(
        "networkmanager interface={interface} state={last}"
    ));
    last
}

/// One USB state read without waiting, for final summaries.
pub fn usb_now(ctx: &Ctx<'_>) -> UsbEthernet {
    match adb::observe(ctx.adb, &ctx.config.phone.serial) {
        Ok((_, state)) => state.ethernet,
        Err(observation) => UsbEthernet::Unknown(observation),
    }
}
