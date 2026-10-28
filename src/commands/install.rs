//! `install` (root): build the `adb-wan` user, ADB key, config, unit and udev files, verify, enable.

use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::fs::{PermissionsExt, chown, lchown};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::account::{self, USER};
use crate::adb::Adb;
use crate::adb::device::Serial;
use crate::adb::server::RealAdb;
use crate::adb::usb_state::UsbEthernet;
use crate::config::{self, Config};
use crate::context::Ctx;
use crate::error::{Error, Exit, Result};
use crate::host::{RealHost, interfaces_of_usb, usb_devices};
use crate::log::Log;
use crate::net::interface::InterfaceName;
use crate::recovery::cycle::{wait_phone, wait_valid_usb};
use crate::recovery::marker::Markers;
use crate::units::service::{BINARY, UNIT_NAME, UNIT_PATH, render_unit};
use crate::wait::SystemClock;

const SYSUSERS_PATH: &str = "/etc/sysusers.d/adb-wan.conf";
const SYSUSERS: &str = "# adb-wan: runs phone WAN recovery (adb-wan.service).\n\
                        u adb-wan - \"ADB WAN recovery\" /var/lib/adb-wan /usr/sbin/nologin\n";
const UDEV_PATH: &str = "/etc/udev/rules.d/70-adb-wan.rules";
const SYSTEMD_SYSUSERS: &str = "/usr/bin/systemd-sysusers";
const UDEVADM: &str = "/usr/bin/udevadm";
const RESTORECON: &str = "/usr/sbin/restorecon";
const TOOL_TIMEOUT: Duration = Duration::from_secs(60);
/// Units of the removed Bash version. Its timer would otherwise still run
/// the old recovery at boot, and its server would hold port 5038.
const LEGACY_UNITS: [&str; 4] = [
    "adb-wan-recover.timer",
    "adb-wan-recover.service",
    "adb-wan-server.service",
    "adb-wan-test@.service",
];
/// Files the Bash version installed. Its logs in /var/log are kept as evidence.
const LEGACY_PATHS: [&str; 12] = [
    "/etc/systemd/system/adb-wan-recover.timer",
    "/etc/systemd/system/adb-wan-recover.service",
    "/etc/systemd/system/adb-wan-server.service",
    "/etc/systemd/system/adb-wan-test@.service",
    "/usr/local/bin/adb-wan-boot",
    "/usr/local/bin/adb-wan-test",
    "/usr/local/bin/adb-wan-recover",
    "/usr/local/bin/adb-wan-host-cycle",
    "/usr/local/bin/adb-ethernet-control",
    "/usr/local/bin/adb-wan-local-check",
    "/etc/adb-ethernet-control/boot.env",
    "/etc/adb-ethernet-control/wan-recovery.env",
];
/// Generous: the user may need to unlock the phone and accept the prompt.
const PHONE_WAIT: Duration = Duration::from_secs(120);

/// How the service user gets an ADB key the phone trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum KeyChoice {
    /// Copy the invoking user's already-authorized key.
    Copy,
    /// Let ADB generate a new key; accept the prompt on the phone.
    New,
}

/// Answers given on the command line.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct InstallArgs {
    /// Show what would be done without changing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Accept detected or supplied answers without asking.
    #[arg(long)]
    pub yes: bool,
    /// Phone USB serial.
    #[arg(long)]
    pub serial: Option<String>,
    /// Host interface of the phone's USB Ethernet.
    #[arg(long)]
    pub interface: Option<String>,
    /// Path to the adb executable (not under /home).
    #[arg(long)]
    pub adb: Option<PathBuf>,
    /// Service that must be active first (repeatable), e.g. your LAN DNS/DHCP.
    #[arg(long = "ready-unit")]
    pub ready_units: Vec<String>,
    /// ADB key for the service user.
    #[arg(long, value_enum)]
    pub key: Option<KeyChoice>,
}

struct Installer<'a> {
    log: &'a Log,
    dry_run: bool,
}

impl Installer<'_> {
    /// Logs a step and runs it unless this is a dry run.
    fn step(&self, what: &str, action: impl FnOnce() -> Result<()>) -> Result<()> {
        if self.dry_run {
            self.log.info(format_args!("would: {what}"));
            return Ok(());
        }
        self.log.info(format_args!("step: {what}"));
        action()
    }
}

/// Runs the installer.
#[allow(
    clippy::too_many_lines,
    reason = "a linear list of install steps; splitting it hides their order"
)]
pub fn run(log: &Log, config_path: &Path, args: &InstallArgs) -> Result<Exit> {
    if !args.dry_run && !account::is_root() {
        return Err(Error::Control(
            "install needs root: sudo ./target/release/adb-wan install (or --dry-run)".into(),
        ));
    }
    let existing = Config::load(config_path).ok();
    let answers = ask_answers(args, existing.as_ref())?;
    let config_text = config::render(
        &answers.serial,
        &answers.interface,
        &answers.adb,
        &answers.units,
    );
    let config = Config::parse(&config_text)?;
    let keep_existing = existing.as_ref().is_some_and(|e| {
        e.phone.serial == config.phone.serial
            && e.wan.interface == config.wan.interface
            && e.adb.binary == config.adb.binary
            && e.readiness.units == config.readiness.units
    });

    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "\nSettings:\n  phone serial   {}\n  WAN interface  {}\n  adb            {}\n  wait for       {}\n  config         {} ({})\n",
        answers.serial,
        answers.interface,
        answers.adb.display(),
        answers.units.join(" "),
        config_path.display(),
        if keep_existing {
            "kept: other settings unchanged"
        } else {
            "written from the example"
        }
    );
    drop(out);
    if !args.yes && !args.dry_run && !confirm("Install and enable adb-wan.service?", false)? {
        return Err(Error::Control("cancelled; nothing changed".into()));
    }

    let installer = Installer {
        log,
        dry_run: args.dry_run,
    };
    let legacy: Vec<&str> = LEGACY_PATHS
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .collect();
    if !legacy.is_empty() {
        installer.step(
            &format!(
                "retire the old Bash install: disable/stop its units, remove {}",
                legacy.join(" ")
            ),
            || remove_legacy(log, &config, &legacy),
        )?;
    }
    installer.step(
        &format!("create system user {USER} ({SYSUSERS_PATH})"),
        || {
            write_file(Path::new(SYSUSERS_PATH), SYSUSERS, 0o644, None)?;
            tool(SYSTEMD_SYSUSERS, &[SYSUSERS_PATH])
        },
    )?;
    let ids = if args.dry_run {
        None
    } else {
        let user = account::lookup(USER)
            .ok_or_else(|| Error::Control(format!("user {USER} was not created")))?;
        Some((user.uid, user.gid))
    };

    installer.step(&format!("install this program as {BINARY}"), install_binary)?;
    if !keep_existing {
        installer.step(
            &format!("write {} (root:{USER} 0640)", config_path.display()),
            || write_file(config_path, &config_text, 0o640, ids),
        )?;
    }
    let rule = udev_rule(&answers.serial);
    installer.step(
        &format!("write {UDEV_PATH} and apply it to the attached phone"),
        || {
            write_file(Path::new(UDEV_PATH), &rule, 0o644, None)?;
            tool(UDEVADM, &["control", "--reload-rules"])?;
            let attr = format!("serial={}", answers.serial);
            tool(
                UDEVADM,
                &[
                    "trigger",
                    "--action=change",
                    "--subsystem-match=usb",
                    "--attr-match",
                    &attr,
                ],
            )
        },
    )?;
    installer.step(
        &format!("prepare {} and the ADB key", config.adb.home.display()),
        || prepare_home(log, &config.adb.home, ids, args),
    )?;
    installer.step(&format!("write {UNIT_PATH} and reload systemd"), || {
        write_file(Path::new(UNIT_PATH), &render_unit(), 0o644, None)?;
        tool(
            &config.paths.systemctl.display().to_string(),
            &["daemon-reload"],
        )
    })?;
    installer.step("check the phone is reachable as the service user", || {
        verify_phone(log, &config, ids)
    })?;
    installer.step(&format!("enable {UNIT_NAME} and run it once now"), || {
        let systemctl = config.paths.systemctl.display().to_string();
        tool(&systemctl, &["enable", UNIT_NAME])?;
        // Restart, not start: a run from earlier in this boot stays active
        // (RemainAfterExit). The once-per-boot marker still prevents a
        // second recovery action.
        tool(&systemctl, &["restart", UNIT_NAME])
    })?;
    if args.dry_run {
        log.info("dry run complete; nothing was changed");
        return Ok(Exit::Success);
    }
    log.info("installed. Logs: journalctl -b -t adb-wan   Records: /var/log/adb-wan/");
    log.info("hardware test (only when you want one): sudo adb-wan test-usb-toggle");
    Ok(Exit::Success)
}

struct Answers {
    serial: Serial,
    interface: InterfaceName,
    adb: PathBuf,
    units: Vec<String>,
}

fn ask_answers(args: &InstallArgs, existing: Option<&Config>) -> Result<Answers> {
    let phones: Vec<_> = usb_devices().into_iter().filter(|d| d.adb).collect();
    let detected_serial = match phones.as_slice() {
        [phone] => Some(phone.serial.clone()),
        _ => None,
    };
    let serial_default = args
        .serial
        .clone()
        .or_else(|| existing.map(|c| c.phone.serial.to_string()))
        .or(detected_serial);
    let serial = Serial::try_from(ask(
        "Phone USB serial (adb devices -l, the line with usb:)",
        serial_default.as_deref(),
        args.yes,
    )?)
    .map_err(Error::Config)?;

    let detected_iface = detect_interface(&serial);
    let iface_default = args
        .interface
        .clone()
        .or_else(|| existing.map(|c| c.wan.interface.to_string()))
        .or(detected_iface);
    let interface = InterfaceName::try_from(ask(
        "WAN interface of the phone (nmcli device; absent while tethering is off)",
        iface_default.as_deref(),
        args.yes,
    )?)
    .map_err(Error::Config)?;

    let adb_default = args
        .adb
        .clone()
        .or_else(|| existing.map(|c| c.adb.binary.clone()))
        .or_else(find_adb)
        .map(|p| p.display().to_string());
    let adb = PathBuf::from(ask("adb executable", adb_default.as_deref(), args.yes)?);
    if adb.starts_with("/home") || adb.starts_with("/root") {
        return Err(Error::Config(format!(
            "{} is inside a home directory, which the service cannot read (ProtectHome); copy it to /usr/local/bin/adb",
            adb.display()
        )));
    }
    if !adb.is_file() {
        return Err(Error::Config(format!("{} does not exist", adb.display())));
    }

    let units = if args.ready_units.is_empty() {
        let default = existing.map_or_else(
            || "NetworkManager.service".to_owned(),
            |c| c.readiness.units.join(" "),
        );
        ask(
            "Services that must be active first, space-separated (add your LAN DNS/DHCP)",
            Some(&default),
            args.yes,
        )?
        .split_whitespace()
        .map(str::to_owned)
        .collect()
    } else {
        args.ready_units.clone()
    };
    Ok(Answers {
        serial,
        interface,
        adb,
        units,
    })
}

fn detect_interface(serial: &Serial) -> Option<String> {
    let device = usb_devices()
        .into_iter()
        .find(|d| d.serial == serial.as_str())?;
    match interfaces_of_usb(&device.path).as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

fn find_adb() -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .map(PathBuf::from)
        .chain([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")])
        .map(|dir| dir.join("adb"))
        .find(|candidate| candidate.is_file() && !candidate.starts_with("/home"))
}

fn ask(question: &str, default: Option<&str>, yes: bool) -> Result<String> {
    if yes {
        return default
            .map(str::to_owned)
            .ok_or_else(|| Error::Config(format!("no answer for: {question}")));
    }
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{question} [{}]: ", default.unwrap_or(""));
    let _ = out.flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| Error::io("reading answer", e))?;
    let answer = line.trim();
    if answer.is_empty() {
        default
            .map(str::to_owned)
            .ok_or_else(|| Error::Config(format!("an answer is required for: {question}")))
    } else {
        Ok(answer.to_owned())
    }
}

fn confirm(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    let answer = ask(
        &format!("{question} ({hint})"),
        Some(if default { "y" } else { "n" }),
        false,
    )?;
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

/// The udev rule giving the service user's group access to this phone only.
/// Matches the serial, which stays the same when USB functions change.
pub fn udev_rule(serial: &Serial) -> String {
    format!(
        "# Generated by adb-wan install: lets the {USER} user open the phone without root.\n\
         # Only sets permissions; never starts anything (that could loop on re-enumeration).\n\
         SUBSYSTEM==\"usb\", ENV{{DEVTYPE}}==\"usb_device\", ATTR{{serial}}==\"{serial}\", GROUP=\"{USER}\", MODE=\"0660\"\n"
    )
}

fn tool(program: &str, args: &[&str]) -> Result<()> {
    let output = crate::process::run(
        &crate::process::Cmd::new(program, TOOL_TIMEOUT).args(args.iter().copied()),
    )?;
    if output.success() {
        Ok(())
    } else {
        Err(Error::Control(format!(
            "{program} {} failed: {}",
            args.join(" "),
            output.summary()
        )))
    }
}

fn write_file(path: &Path, text: &str, mode: u32, owner: Option<(u32, u32)>) -> Result<()> {
    let context = |e| Error::io(format!("writing {}", path.display()), e);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(context)?;
    }
    let tmp = path.with_extension("adb-wan-tmp");
    fs::write(&tmp, text).map_err(context)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)).map_err(context)?;
    if let Some((_, gid)) = owner {
        chown(&tmp, Some(0), Some(gid)).map_err(context)?;
    }
    fs::rename(&tmp, path).map_err(context)?;
    relabel(path);
    Ok(())
}

/// Restores `SELinux` labels; skipped quietly where `restorecon` is absent.
fn relabel(path: &Path) {
    if Path::new(RESTORECON).exists() {
        let _ = tool(RESTORECON, &["-RF", &path.display().to_string()]);
    }
}

fn remove_legacy(log: &Log, config: &Config, paths: &[&str]) -> Result<()> {
    let systemctl = config.paths.systemctl.display().to_string();
    for unit in LEGACY_UNITS {
        // Missing or template units fail here; that is fine, the files go next.
        if let Err(e) = tool(&systemctl, &["disable", "--now", unit]) {
            log.debug(format_args!("legacy unit {unit}: {e}"));
        }
    }
    for path in paths {
        let path = Path::new(path);
        let removed = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        removed.map_err(|e| Error::io(format!("removing {}", path.display()), e))?;
        log.info(format_args!("removed {}", path.display()));
    }
    tool(&systemctl, &["daemon-reload"])
}

fn install_binary() -> Result<()> {
    let current = std::env::current_exe().map_err(|e| Error::io("finding this program", e))?;
    let target = Path::new(BINARY);
    if fs::canonicalize(&current).ok() == fs::canonicalize(target).ok() {
        return Ok(());
    }
    let context = |e| Error::io(format!("installing {BINARY}"), e);
    let tmp = target.with_extension("tmp");
    fs::copy(&current, &tmp).map_err(context)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(context)?;
    fs::rename(&tmp, target).map_err(context)?;
    relabel(target);
    Ok(())
}

fn prepare_home(log: &Log, home: &Path, ids: Option<(u32, u32)>, args: &InstallArgs) -> Result<()> {
    let android = home.join(".android");
    let context = |e| Error::io(format!("preparing {}", android.display()), e);
    fs::create_dir_all(&android).map_err(context)?;
    let key = android.join("adbkey");
    if key.exists() {
        log.info("service user already has an ADB key; kept");
    } else if let Some(source) = sudo_user_key() {
        let choice = match args.key {
            Some(choice) => choice,
            None if args.yes => KeyChoice::Copy,
            None => {
                if confirm(
                    &format!(
                        "Copy the ADB key from {} (the phone already trusts it)?",
                        source.display()
                    ),
                    true,
                )? {
                    KeyChoice::Copy
                } else {
                    KeyChoice::New
                }
            }
        };
        if choice == KeyChoice::Copy {
            fs::copy(&source, &key).map_err(context)?;
            fs::copy(source.with_extension("pub"), android.join("adbkey.pub")).map_err(context)?;
            log.info("copied the authorized ADB key to the service user");
        } else {
            log.info("a new key will be generated; accept \"Allow USB debugging\" on the phone");
        }
    } else {
        log.info(
            "no existing key found; a new one will be generated; accept the prompt on the phone",
        );
    }
    // The Bash version ran its ADB server as root here; hand everything
    // (key, known hosts, ...) to the service user.
    own_tree(home, ids).map_err(context)?;
    relabel(home);
    Ok(())
}

/// Makes `dir` and everything below private to the service user. Symlinks
/// are re-owned themselves, never followed.
fn own_tree(dir: &Path, ids: Option<(u32, u32)>) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(dir)?;
    if let Some((uid, gid)) = ids {
        lchown(dir, Some(uid), Some(gid))?;
    }
    if meta.is_symlink() {
        return Ok(());
    }
    let mode = if meta.is_dir() { 0o700 } else { 0o600 };
    fs::set_permissions(dir, fs::Permissions::from_mode(mode))?;
    if meta.is_dir() {
        for entry in fs::read_dir(dir)? {
            own_tree(&entry?.path(), ids)?;
        }
    }
    Ok(())
}

/// The invoking user's ADB private key, when run through sudo.
fn sudo_user_key() -> Option<PathBuf> {
    // SUDO_USER identifies who ran sudo; it is not a program setting.
    let user = std::env::var("SUDO_USER").ok()?;
    let key = account::lookup(&user)?.home.join(".android/adbkey");
    key.is_file().then_some(key)
}

fn verify_phone(log: &Log, config: &Config, ids: Option<(u32, u32)>) -> Result<()> {
    let adb = RealAdb::new(&config.adb, ids);
    let host = RealHost::new(config);
    let mut patient = config.clone();
    patient.timing.phone_timeout_secs =
        config::Secs::try_from(PHONE_WAIT.as_secs()).map_err(Error::Config)?;
    let markers = Markers::new(std::env::temp_dir());
    let ctx = Ctx {
        config: &patient,
        adb: &adb,
        host: &host,
        clock: &SystemClock,
        log,
        markers: &markers,
    };
    log.info("if the phone asks \"Allow USB debugging?\", unlock it and tap Allow (tick \"Always allow\")");
    adb.start_server()?;
    let result = wait_phone(&ctx).and_then(|_| wait_valid_usb(&ctx));
    adb.stop_server();
    let (_, state) = result.map_err(|e| {
        Error::Control(format!(
            "{e}. Fix this and run install again; the service was not enabled"
        ))
    })?;
    let judgement = match state.ethernet {
        UsbEthernet::On(_) => "on: nothing to do while WAN is healthy",
        UsbEthernet::Off => "off: boot recovery will request it once",
        UsbEthernet::Unknown(_) => "unknown: boot recovery will not act on it",
    };
    log.info(format_args!(
        "phone reachable; sys.usb.state={} → USB Ethernet {judgement}",
        state.raw
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udev_rule_only_sets_permissions_for_this_phone() {
        let serial = Serial::try_from("FAKE0SERIAL".to_owned()).unwrap();
        let rule = udev_rule(&serial);
        assert!(rule.contains("ATTR{serial}==\"FAKE0SERIAL\""));
        assert!(rule.contains("GROUP=\"adb-wan\", MODE=\"0660\""));
        assert!(!rule.contains("SYSTEMD_WANTS"));
        assert!(!rule.contains("RUN"));
    }

    #[test]
    fn sysusers_declares_a_system_user_without_login() {
        assert!(SYSUSERS.contains("u adb-wan -"));
        assert!(SYSUSERS.contains("/usr/sbin/nologin"));
    }
}
