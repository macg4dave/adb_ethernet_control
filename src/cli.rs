//! Command-line definition: subcommands and flags, no behavior.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::commands::install::InstallArgs;
use crate::config::DEFAULT_PATH;

/// Recover a USB-tethered Android phone WAN after host startup.
#[derive(Debug, Parser)]
#[command(name = "adb-wan", version)]
pub struct Cli {
    /// Config file.
    #[arg(long, global = true, default_value = DEFAULT_PATH)]
    pub config: PathBuf,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Read-only report of the host interface, WAN and phone USB state.
    Status,
    /// Read-only WAN health check: exit 0 up, 2 down.
    Check,
    /// Once-per-boot check and bounded recovery (run by adb-wan.service).
    Boot,
    /// Enable-only fallback after an unfinished USB off/on (`ExecStopPost=`).
    Restore,
    /// Install and enable everything (run with sudo).
    Install(InstallArgs),
    /// Hardware test: switch USB Ethernet off and on once, recording every step.
    TestUsbToggle {
        /// Internal: set when already running inside the transient unit.
        #[arg(long, hide = true)]
        supervised: bool,
    },
    /// Hardware test: with USB tethering switched off on the phone, wait for ADB
    /// and enable rndis once, recording every step.
    TestUsbEnable {
        /// Internal: set when already running inside the transient unit.
        #[arg(long, hide = true)]
        supervised: bool,
    },
}
