//! One module per subcommand; this file builds the real context and dispatches.

pub mod boot;
pub mod check;
pub mod install;
pub mod restore;
pub mod status;
pub mod usb_toggle;

use std::path::Path;

use crate::account;
use crate::adb::server::RealAdb;
use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::host::RealHost;
use crate::log::{Log, quote};
use crate::record::{Record, boot_id};
use crate::recovery::marker::Markers;
use crate::wait::SystemClock;

/// Runs the selected command against the real system.
pub fn run(cli: &Cli, log: &Log) -> Result<Exit> {
    let flow = match &cli.command {
        Command::Install(args) => return install::run(log, &cli.config, args),
        Command::TestUsbToggle { supervised: false } => {
            return usb_toggle::launch(&Config::load(&cli.config)?, log);
        }
        Command::Status => Flow::Status,
        Command::Check => Flow::Check,
        Command::Boot => Flow::Boot,
        Command::Restore => Flow::Restore,
        Command::TestUsbToggle { supervised: true } => Flow::Test,
    };
    let config = Config::load(&cli.config)?;
    let record = flow
        .record_kind()
        .and_then(|kind| open_record(log, &config, kind));
    start_line(log, flow.name(), &cli.config, &config);

    let adb = RealAdb::new(&config.adb, account::adb_server_identity());
    let host = RealHost::new(&config);
    let markers = Markers::new(&config.paths.runtime_dir);
    let ctx = Ctx {
        config: &config,
        adb: &adb,
        host: &host,
        clock: &SystemClock,
        log,
        markers: &markers,
    };
    match flow {
        Flow::Status => status::run(&ctx),
        Flow::Check => check::run(&ctx),
        Flow::Boot => boot::run(&ctx, record.as_ref()),
        Flow::Restore => restore::run(&ctx),
        Flow::Test => usb_toggle::run(&ctx, record.as_ref()),
    }
}

/// Commands that run against a loaded config and the real system.
#[derive(Clone, Copy)]
enum Flow {
    Status,
    Check,
    Boot,
    Restore,
    Test,
}

impl Flow {
    fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Check => "check",
            Self::Boot => "boot",
            Self::Restore => "restore",
            Self::Test => "test-usb-toggle",
        }
    }

    /// Runs that keep a record under `/var/log/adb-wan/`.
    fn record_kind(self) -> Option<&'static str> {
        match self {
            Self::Status | Self::Check => None,
            Self::Boot => Some("boot"),
            Self::Restore => Some("restore"),
            Self::Test => Some("test"),
        }
    }
}

/// Creates a per-run record and mirrors the log into it. A missing log
/// directory (e.g. a development run) only costs the record.
fn open_record(log: &Log, config: &Config, kind: &str) -> Option<Record> {
    let opened = Record::create(&config.paths.log_dir, kind).and_then(|record| {
        let events = record.events()?;
        Ok((record, events))
    });
    match opened {
        Ok((record, events)) => {
            log.attach_record(Box::new(events));
            Some(record)
        }
        Err(e) => {
            log.warn(format_args!(
                "no run record error={}",
                quote(&e.to_string())
            ));
            None
        }
    }
}

/// First line of every run: enough to tell runs and settings apart.
fn start_line(log: &Log, command: &str, path: &Path, config: &Config) {
    log.info(format_args!(
        "adb-wan start command={command} version={} boot_id={} config={} serial={} interface={} usb_function={} cycle_enabled={} adb_port={} uid={}",
        env!("CARGO_PKG_VERSION"),
        boot_id(),
        path.display(),
        config.phone.serial.redacted(),
        config.wan.interface,
        config.phone.usb_function,
        config.recovery.usb_cycle_enabled,
        config.adb.server_port,
        account::effective_uid().map_or_else(|| "?".into(), |u| u.to_string()),
    ));
}
