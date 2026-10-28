//! `adb-wan` entry point: parse the CLI, run one command, map the result to an exit code.

use std::process::ExitCode;

use adb_ethernet_control::cli::Cli;
use adb_ethernet_control::commands;
use adb_ethernet_control::error::exit_code;
use adb_ethernet_control::log::{Log, quote};
use clap::Parser;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let log = Log::stderr();
    let result = commands::run(&cli, &log);
    if let Err(e) = &result {
        log.error(format_args!("exit=1 error={}", quote(&e.to_string())));
    }
    exit_code(&result)
}
