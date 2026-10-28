//! The `adb-wan.service` property list, rendered to the unit file and to the test's `systemd-run` call.

use crate::config;

/// Installed program path. Outside `/home`: `SELinux` refused to execute a
/// binary from the checkout (`203/EXEC`).
pub const BINARY: &str = "/usr/local/bin/adb-wan";
/// The one service.
pub const UNIT_NAME: &str = "adb-wan.service";
/// Where `install` writes the unit.
pub const UNIT_PATH: &str = "/etc/systemd/system/adb-wan.service";
/// Name of the transient unit for the manual hardware test.
pub const TEST_UNIT: &str = "adb-wan-test";
/// The committed copy of the rendered unit.
pub const COMMITTED_UNIT: &str = include_str!("../../systemd/system/adb-wan.service");

/// One `[Service]` setting, with the reason for it when not obvious.
struct Property {
    key: &'static str,
    value: String,
    why: Option<&'static str>,
}

fn property(key: &'static str, value: impl Into<String>, why: Option<&'static str>) -> Property {
    Property {
        key,
        value: value.into(),
        why,
    }
}

fn command(subcommand: &str) -> String {
    format!("{BINARY} --config {} {subcommand}", config::DEFAULT_PATH)
}

/// Settings shared by the boot service and the manual test, so the two
/// always run with the same user, directories, hardening and restore.
fn shared() -> Vec<Property> {
    vec![
        property("User", "adb-wan", None),
        property("Group", "adb-wan", None),
        property(
            "Environment",
            "HOME=/var/lib/adb-wan",
            Some("ADB keeps its authorized key in HOME; this is not a program setting."),
        ),
        property("StateDirectory", "adb-wan", None),
        property("StateDirectoryMode", "0700", None),
        property("LogsDirectory", "adb-wan", None),
        property("LogsDirectoryMode", "0750", None),
        property(
            "RuntimeDirectory",
            "adb-wan",
            Some("Markers: once-per-boot guard and restore-pending. Kept until reboot."),
        ),
        property("RuntimeDirectoryMode", "0750", None),
        property("RuntimeDirectoryPreserve", "yes", None),
        property(
            "ExecStopPost",
            command("restore"),
            Some("Independent restore: runs even if the main process crashes or times out."),
        ),
        property("TimeoutStartSec", "15min", None),
        property("TimeoutStopSec", "5min", None),
        property(
            "AmbientCapabilities",
            "CAP_NET_RAW",
            Some("Only for the ping bound to the phone interface."),
        ),
        property("CapabilityBoundingSet", "CAP_NET_RAW", None),
        property("NoNewPrivileges", "yes", None),
        property("UMask", "0077", None),
        property("PrivateTmp", "yes", None),
        property(
            "ProtectSystem",
            "strict",
            Some(
                "No PrivateDevices/PrivateNetwork: they would hide the USB phone and the WAN interface.",
            ),
        ),
        property("ProtectHome", "yes", None),
        property("ProtectKernelTunables", "yes", None),
        property("ProtectKernelModules", "yes", None),
        property("ProtectControlGroups", "yes", None),
        property("RestrictSUIDSGID", "yes", None),
        property("LockPersonality", "yes", None),
        property("RestrictRealtime", "yes", None),
        property("SyslogIdentifier", "adb-wan", None),
    ]
}

fn push(text: &mut String, p: &Property) {
    if let Some(why) = p.why {
        text.push_str("# ");
        text.push_str(why);
        text.push('\n');
    }
    text.push_str(p.key);
    text.push('=');
    text.push_str(&p.value);
    text.push('\n');
}

/// The complete `adb-wan.service` file.
pub fn render_unit() -> String {
    let mut text = String::from(
        "# Generated from src/units/service.rs; installed by `adb-wan install`.\n\
         [Unit]\n\
         Description=Recover the phone USB Ethernet WAN once after startup\n\
         Wants=network-online.target NetworkManager.service\n\
         After=network-online.target NetworkManager.service\n\
         \n\
         [Service]\n",
    );
    push(&mut text, &property("Type", "oneshot", None));
    push(
        &mut text,
        &property(
            "RemainAfterExit",
            "yes",
            Some("Once per boot. No Restart=: at most one recovery action."),
        ),
    );
    push(&mut text, &property("ExecStart", command("boot"), None));
    for p in shared() {
        push(&mut text, &p);
    }
    text.push_str("\n[Install]\nWantedBy=multi-user.target\n");
    text
}

/// `systemd-run` arguments that run the manual test as a transient service
/// with the boot service's settings. Never a timer.
pub fn test_run_args() -> Vec<String> {
    let mut args = vec![
        format!("--unit={TEST_UNIT}"),
        "--wait".into(),
        "--collect".into(),
        "--property=Type=oneshot".into(),
    ];
    args.extend(
        shared()
            .into_iter()
            .map(|p| format!("--property={}={}", p.key, p.value)),
    );
    args.extend([
        BINARY.into(),
        "--config".into(),
        config::DEFAULT_PATH.into(),
        "test-usb-toggle".into(),
        "--supervised".into(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directives(unit: &str) -> Vec<(&str, &str)> {
        unit.lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with('['))
            .filter_map(|l| l.split_once('='))
            .collect()
    }

    #[test]
    fn committed_unit_matches_rendered() {
        assert_eq!(
            COMMITTED_UNIT,
            render_unit(),
            "regenerate systemd/system/adb-wan.service from render_unit()"
        );
    }

    #[test]
    fn unit_follows_the_rules() {
        let unit = render_unit();
        let d = directives(&unit);
        let has = |k: &str, v: &str| d.contains(&(k, v));
        let any = |k: &str| d.iter().any(|(key, _)| *key == k);
        assert!(has("User", "adb-wan"));
        assert!(has("Type", "oneshot"));
        assert!(has(
            "ExecStart",
            "/usr/local/bin/adb-wan --config /etc/adb-ethernet-control/config.toml boot"
        ));
        assert!(has(
            "ExecStopPost",
            "/usr/local/bin/adb-wan --config /etc/adb-ethernet-control/config.toml restore"
        ));
        assert!(has("RuntimeDirectoryPreserve", "yes"));
        assert!(has("NoNewPrivileges", "yes"));
        assert!(has("ProtectSystem", "strict"));
        for forbidden in [
            "Restart",
            "EnvironmentFile",
            "PrivateDevices",
            "PrivateNetwork",
            "PrivateUsers",
            "IPAddressDeny",
            "SuccessExitStatus",
            "OnBootSec",
        ] {
            assert!(!any(forbidden), "{forbidden} must not be set");
        }
        assert_eq!(d.iter().filter(|(k, _)| *k == "Environment").count(), 1);
        assert!(!unit.contains("${"));
    }

    #[test]
    fn test_unit_shares_every_service_setting() {
        let args = test_run_args();
        for p in shared() {
            let wanted = format!("--property={}={}", p.key, p.value);
            assert!(args.contains(&wanted), "missing {wanted}");
        }
        assert!(args.contains(&"--wait".to_owned()));
        assert!(
            !args.iter().any(|a| a.starts_with("--on-")),
            "never a timer"
        );
        assert_eq!(args.last().map(String::as_str), Some("--supervised"));
    }
}
