# ADB Ethernet Control

Recover a RHEL host's main WAN, supplied by a USB-tethered Android phone,
after a host restart leaves USB tethering off or unusable.

**Status: rewrite in progress.** The earlier Bash implementation never
recovered the WAN automatically and has been removed. The new version is
Rust + systemd only; no working program exists yet.

## Planned design

- One binary, `adb-wan`, with subcommands `status`, `check`, `boot`,
  `restore`, `install` and `test-usb-toggle`.
- Setup: `cargo build --release && sudo ./target/release/adb-wan install`.
  Only the installer runs as root; everything else runs as the system user
  `adb-wan`.
- Hardware test: `sudo adb-wan test-usb-toggle` switches USB Ethernet off and
  on once under the system manager and records every step.
- Boot runs are checked by reading `journalctl -b -t adb-wan`; each run ends
  with a one-line `result=...` summary.
- One config file: `/etc/adb-ethernet-control/config.toml`.
- One systemd service, `adb-wan.service`, running once per boot as the
  system user `adb-wan`. No timers: the program acts when the services,
  phone and interface it needs are observably ready, with a deadline on
  every wait.
- A healthy WAN is never touched. At most one recovery action per boot. If a
  run dies mid-recovery, the service's `ExecStopPost=` re-enables USB
  Ethernet.

## Documentation

- [Rules for contributors and AI agents](AGENTS.md)
- [Hardware findings from the Bash version](docs/legacy-findings.md)

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests are Rust only and run without a phone, root, or network changes.
