# ADB Ethernet Control

Recover a RHEL host's main WAN, supplied by a USB-tethered Android phone,
after a host restart leaves USB tethering off or unusable.

**Status:** the Rust program is written and passes its tests (no phone
needed). It has not yet been installed or run on the real phone. An
earlier Bash version never recovered the WAN automatically and has been
removed; see [what was learned](docs/legacy-findings.md).

## How it works

- One binary, `adb-wan`. One config file,
  `/etc/adb-ethernet-control/config.toml`. One service, `adb-wan.service`,
  running once per boot as the system user `adb-wan`.
- No timers: it waits for NetworkManager (and any services you list), then
  for the WAN. Every wait has a deadline.
- A healthy WAN is never touched and ADB is not even started.
- If WAN is down, it starts its own ADB server, reads the phone's
  `sys.usb.state` and takes at most one action per boot:
  - USB Ethernet off → enable it once;
  - USB Ethernet on → one off/on cycle, only if
    `recovery.usb_cycle_enabled = true` (off by default).
- If a run dies during an off/on, the service's `ExecStopPost=` re-enables
  USB Ethernet.

## Install

```bash
cargo build --release
./target/release/adb-wan install --dry-run   # see what it would do
sudo ./target/release/adb-wan install
```

The installer detects the phone serial and interface, asks you to confirm,
retires the old Bash install, creates the `adb-wan` user, hands it the ADB
key the phone already trusts, writes the config, udev rule and unit, checks
the phone is reachable, and enables the service.

## Use

```bash
adb-wan status                       # read-only report
adb-wan check                        # WAN health: exit 0 up, 2 down
sudo adb-wan test-usb-toggle         # hardware test: USB Ethernet off, then on, recorded
journalctl -b -t adb-wan             # what the boot run did
ls /var/log/adb-wan/                 # per-run records (boot-*, test-*, restore-*)
```

Each boot run ends with one line such as
`result=passed action=enable changed_by_us=yes usb=on wan=up elapsed_s=41`.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests are Rust only and run without a phone, root, or network changes.
Rules for contributors and AI agents: [AGENTS.md](AGENTS.md).
