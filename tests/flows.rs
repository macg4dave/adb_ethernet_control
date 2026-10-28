//! Boot and restore flows against an in-process fake phone and host.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::struct_excessive_bools,
    clippy::format_push_string,
    reason = "test fixtures: failing loudly is the point, and the fake world is a bag of switches"
)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adb_ethernet_control::adb::Adb;
use adb_ethernet_control::adb::device::{Serial, TransportId};
use adb_ethernet_control::adb::usb_state::NetFunction;
use adb_ethernet_control::commands::{boot, restore};
use adb_ethernet_control::config::Config;
use adb_ethernet_control::context::Ctx;
use adb_ethernet_control::error::{Error, Exit, Result};
use adb_ethernet_control::host::Host;
use adb_ethernet_control::log::{Log, LogBuffer};
use adb_ethernet_control::net::interface::{InterfaceName, InterfaceStatus};
use adb_ethernet_control::process::CommandOutput;
use adb_ethernet_control::recovery::marker::Markers;
use adb_ethernet_control::wait::FakeClock;

const CONFIG: &str = "[phone]\nserial = \"FAKE0SERIAL\"\n[wan]\ninterface = \"usb0\"\n";

/// Shared simulated phone + host. Switching USB functions re-enumerates the
/// phone (new transport ID) and makes the WAN interface appear/disappear.
struct World {
    usb_state: String,
    transport: u32,
    listed: bool,
    server_fails: bool,
    server_started: bool,
    /// Whether an "on" request (rndis/ncm) changes the phone's state; "off" always does.
    on_takes_effect: bool,
    /// Whether turning USB Ethernet on brings working WAN.
    wan_follows_usb: bool,
    attached_on_usb: bool,
    unit_active: bool,
    iface: bool,
    ipv4: bool,
    probe: bool,
    commands: Vec<String>,
}

impl World {
    fn healthy() -> Self {
        Self {
            usb_state: "rndis,adb".into(),
            transport: 5,
            listed: true,
            server_fails: false,
            server_started: false,
            on_takes_effect: true,
            wan_follows_usb: true,
            attached_on_usb: true,
            unit_active: true,
            iface: true,
            ipv4: true,
            probe: true,
            commands: Vec::new(),
        }
    }

    fn wan_down(mut self) -> Self {
        self.iface = false;
        self.ipv4 = false;
        self.probe = false;
        self
    }

    fn usb(mut self, state: &str) -> Self {
        self.usb_state = state.into();
        self
    }
}

#[derive(Clone)]
struct Fake(Rc<RefCell<World>>);

impl Adb for Fake {
    fn start_server(&self) -> Result<()> {
        let mut w = self.0.borrow_mut();
        if w.server_fails {
            return Err(Error::Control("ADB server exited during startup".into()));
        }
        w.server_started = true;
        Ok(())
    }

    fn devices(&self) -> Result<String> {
        let w = self.0.borrow();
        assert!(w.server_started, "devices() before start_server()");
        let mut text = String::from("List of devices attached\n");
        if w.listed {
            text.push_str(&format!(
                "FAKE0SERIAL device usb:1-2 product:p model:m device:d transport_id:{}\n",
                w.transport
            ));
        }
        Ok(text)
    }

    fn shell(&self, transport: TransportId, args: &[&str]) -> Result<CommandOutput> {
        let mut w = self.0.borrow_mut();
        assert_eq!(transport.0, w.transport, "stale transport used");
        match args {
            ["getprop", "sys.usb.state"] => Ok(CommandOutput::ok(format!("{}\r\n", w.usb_state))),
            ["getprop", _] => Ok(CommandOutput::ok("x\n")),
            ["svc", "usb", "setFunctions", functions] => {
                w.commands.push((*functions).to_owned());
                if w.on_takes_effect || NetFunction::parse(functions).is_none() {
                    w.usb_state = format!("{functions},adb");
                    w.transport += 1;
                    let on = NetFunction::parse(functions).is_some();
                    w.iface = on;
                    w.ipv4 = on && w.wan_follows_usb;
                    w.probe = on && w.wan_follows_usb;
                }
                // The real phone's response is often lost during re-enumeration.
                Ok(CommandOutput::failed(255, "error: closed"))
            }
            other => panic!("unexpected phone command {other:?}"),
        }
    }

    fn stop_server(&self) {
        self.0.borrow_mut().server_started = false;
    }
}

impl Host for Fake {
    fn unit_active(&self, _unit: &str) -> Result<bool> {
        Ok(self.0.borrow().unit_active)
    }

    fn interface(&self, _name: &InterfaceName) -> Result<InterfaceStatus> {
        let w = self.0.borrow();
        Ok(InterfaceStatus {
            present: w.iface,
            ipv4: (w.iface && w.ipv4).then(|| "192.168.42.17/24".to_owned()),
            driver: w.iface.then(|| "rndis_host".to_owned()),
        })
    }

    fn probe(&self, _name: &InterfaceName) -> Result<bool> {
        Ok(self.0.borrow().probe)
    }

    fn usb_serial_present(&self, _serial: &Serial) -> bool {
        self.0.borrow().attached_on_usb
    }
}

struct Run {
    outcome: Result<Exit>,
    world: Rc<RefCell<World>>,
    log: String,
    markers_dir: PathBuf,
}

impl Run {
    fn exit(&self) -> Exit {
        match &self.outcome {
            Ok(exit) => *exit,
            Err(e) => panic!("expected an exit, got error {e}\n{}", self.log),
        }
    }

    fn error(&self) -> String {
        match &self.outcome {
            Ok(exit) => panic!("expected an error, got {exit:?}"),
            Err(e) => e.to_string(),
        }
    }

    fn commands(&self) -> Vec<String> {
        self.world.borrow().commands.clone()
    }

    fn markers(&self) -> Markers {
        Markers::new(&self.markers_dir)
    }

    fn summary(&self) -> &str {
        self.log
            .lines()
            .find(|l| l.contains(">result="))
            .unwrap_or_else(|| panic!("no summary line in:\n{}", self.log))
    }
}

fn markers_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("adb-wan-flow-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run_with(
    name: &str,
    world: World,
    extra_config: &str,
    prepare: impl FnOnce(&Markers),
    flow: impl FnOnce(&Ctx<'_>) -> Result<Exit>,
) -> Run {
    let config = Config::parse(&format!("{CONFIG}{extra_config}")).unwrap();
    let world = Rc::new(RefCell::new(world));
    let fake = Fake(world.clone());
    let clock = FakeClock::new();
    let (log, buffer): (Log, LogBuffer) = Log::memory();
    let dir = markers_dir(name);
    let markers = Markers::new(&dir);
    prepare(&markers);
    let ctx = Ctx {
        config: &config,
        adb: &fake,
        host: &fake,
        clock: &clock,
        log: &log,
        markers: &markers,
    };
    let exit = flow(&ctx);
    assert!(!world.borrow().server_started, "ADB server left running");
    Run {
        outcome: exit,
        world,
        log: buffer.contents(),
        markers_dir: dir,
    }
}

fn boot_run(name: &str, world: World, extra_config: &str) -> Run {
    run_with(
        name,
        world,
        extra_config,
        |_| {},
        |ctx| boot::run(ctx, None),
    )
}

const CYCLE_ON: &str = "[recovery]\nusb_cycle_enabled = true\n";

#[test]
fn healthy_wan_is_never_touched() {
    let run = boot_run("healthy", World::healthy(), CYCLE_ON);
    assert_eq!(run.exit(), Exit::Success);
    assert!(run.commands().is_empty());
    assert!(run.log.contains("ADB not started"));
    assert!(!run.log.contains("devices"));
    assert!(
        run.summary()
            .contains("result=healthy-no-action action=none changed_by_us=no")
    );
    assert_eq!(run.markers().acted_this_boot(), None);
}

#[test]
fn usb_off_is_enabled_once_and_wan_recovers() {
    let world = World::healthy()
        .wan_down()
        .usb("hisuite,mtp,mass_storage,adb");
    let run = boot_run("enable", world, "");
    assert_eq!(run.exit(), Exit::Success);
    assert_eq!(run.commands(), ["rndis"]);
    assert!(
        run.summary()
            .contains("result=passed action=enable changed_by_us=yes usb=on wan=up")
    );
    assert_eq!(run.markers().acted_this_boot().as_deref(), Some("enable"));
    assert_eq!(run.markers().restore_pending(), None);
}

#[test]
fn usb_on_with_cycling_disabled_changes_nothing_and_exits_2() {
    let run = boot_run("cycle-off", World::healthy().wan_down(), "");
    assert_eq!(run.exit(), Exit::Failed);
    assert!(run.commands().is_empty());
    assert!(run.log.contains("usb_cycle_enabled is false"));
    assert!(
        run.summary()
            .contains("result=failed action=none changed_by_us=no usb=on wan=down")
    );
}

#[test]
fn usb_on_with_cycling_enabled_cycles_once_and_disarms_restore() {
    let mut world = World::healthy().wan_down();
    world.iface = true;
    let run = boot_run("cycle", world, CYCLE_ON);
    assert_eq!(run.exit(), Exit::Success);
    assert_eq!(run.commands(), ["mtp", "rndis"]);
    assert!(run.log.contains("restore armed function=rndis"));
    assert!(run.log.contains("restore disarmed"));
    assert_eq!(run.markers().restore_pending(), None);
    assert!(
        run.summary()
            .contains("result=passed action=cycle changed_by_us=yes")
    );
}

#[test]
fn cycle_that_does_not_come_back_leaves_restore_armed() {
    let mut world = World::healthy().wan_down();
    world.on_takes_effect = false;
    let run = boot_run("cycle-stuck", world, CYCLE_ON);
    assert_eq!(run.exit(), Exit::Failed);
    assert_eq!(
        run.commands(),
        ["mtp", "rndis"],
        "exactly one off and one on"
    );
    assert_eq!(run.markers().restore_pending(), Some(NetFunction::Rndis));
    assert!(run.log.contains("restore stays armed"));
}

#[test]
fn second_run_in_the_same_boot_never_acts_again() {
    let world = World::healthy().wan_down().usb("mtp,adb");
    let run = run_with(
        "acted",
        world,
        CYCLE_ON,
        |m| m.mark_acted("enable").unwrap(),
        |ctx| boot::run(ctx, None),
    );
    assert_eq!(run.exit(), Exit::Failed);
    assert!(run.commands().is_empty());
    assert!(run.log.contains("already acted this boot action=enable"));
}

#[test]
fn wan_that_stays_down_after_enable_is_a_single_attempt() {
    let mut world = World::healthy().wan_down().usb("mtp,adb");
    world.wan_follows_usb = false;
    let run = boot_run("carrier-outage", world, CYCLE_ON);
    assert_eq!(run.exit(), Exit::Failed);
    assert_eq!(run.commands(), ["rndis"]);
    assert!(run.log.contains("stopping without further actions"));
    assert!(
        run.summary()
            .contains("result=failed action=enable changed_by_us=yes usb=on wan=down")
    );
}

#[test]
fn absent_phone_is_a_control_failure_with_a_hint() {
    let mut world = World::healthy().wan_down().usb("mtp,adb");
    world.listed = false;
    let run = boot_run("absent", world, "");
    assert!(run.outcome.is_err());
    assert!(run.commands().is_empty());
    assert!(
        run.log.contains("another ADB"),
        "conflict hint missing:\n{}",
        run.log
    );
    assert!(run.summary().contains("result=error"));
}

#[test]
fn unknown_usb_state_is_never_acted_on() {
    for (i, state) in ["", "mtp", "rndis,ncm,adb"].into_iter().enumerate() {
        let world = World::healthy().wan_down().usb(state);
        let run = boot_run(&format!("unknown-{i}"), world, CYCLE_ON);
        assert!(run.outcome.is_err(), "{state:?}");
        assert!(run.commands().is_empty(), "{state:?}");
    }
}

#[test]
fn readiness_timeout_stops_before_adb() {
    let mut world = World::healthy().wan_down();
    world.unit_active = false;
    world.server_fails = true;
    let run = boot_run("readiness", world, "");
    let err = run.error();
    assert!(err.contains("host services not ready"), "{err}");
    assert!(run.commands().is_empty());
}

#[test]
fn adb_server_failure_changes_nothing() {
    let mut world = World::healthy().wan_down().usb("mtp,adb");
    world.server_fails = true;
    let run = boot_run("server", world, "");
    assert!(run.outcome.is_err());
    assert!(run.commands().is_empty());
}

#[test]
fn every_stage_logs_its_end() {
    let world = World::healthy().wan_down().usb("mtp,adb");
    let run = boot_run("stages", world, "");
    for stage in [
        "readiness",
        "initial-wan",
        "adb-server",
        "phone",
        "usb-state",
        "enable",
        "final-checks",
    ] {
        assert!(
            run.log.contains(&format!("stage end name={stage} ")),
            "missing end of {stage}"
        );
    }
}

#[test]
fn restore_without_pending_marker_does_nothing() {
    let world = World::healthy().wan_down().usb("mtp,adb");
    let run = run_with("restore-none", world, "", |_| {}, restore::run);
    assert_eq!(run.exit(), Exit::Success);
    assert!(run.commands().is_empty());
    assert!(run.log.contains("no restore pending"));
}

#[test]
fn pending_restore_reenables_and_clears() {
    let world = World::healthy().wan_down().usb("mtp,adb");
    let run = run_with(
        "restore",
        world,
        "",
        |m| m.set_restore_pending(NetFunction::Rndis).unwrap(),
        restore::run,
    );
    assert_eq!(run.exit(), Exit::Success);
    assert_eq!(run.commands(), ["rndis"], "enable only, never off");
    assert_eq!(run.markers().restore_pending(), None);
}
