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
- No timers or grace periods. systemd starts it after NetworkManager and any
  services you list; then it acts on what it sees: WAN up → nothing to do;
  USB Ethernet off → enable now; on → wait for NetworkManager to finish, then
  judge.
- A healthy WAN is never touched and ADB is not even started.
- If WAN is down, it starts its own ADB server, reads the phone's
  `sys.usb.state` and takes at most one action per boot:
  - USB Ethernet off → enable it once;
  - USB Ethernet on → one off/on cycle, only if
    `recovery.usb_cycle_enabled = true` (off by default).
- If a run dies during an off/on, the service's `ExecStopPost=` re-enables
  USB Ethernet.

## Phone setup (required)

In the phone's Developer options:

- **USB debugging**: on.
- **Allow ADB debugging in charge only mode**: on. Without it, this HONOR
  phone turns ADB off together with USB tethering, and nothing can switch
  tethering back on. Unplug and replug the cable after changing it.
- Accept **Allow USB debugging** for this computer with **Always allow**.

Do not leave another ADB server running on the host (a desktop `adb` on port
5037): only one server can hold the phone. Stop it with `adb kill-server`.

## Install

```bash
cargo build --release
./target/release/adb-wan install --dry-run   # see what it would do
sudo ./target/release/adb-wan install
```

The installer detects the phone serial and interface, asks you to confirm,
creates the `adb-wan` user, hands it the ADB key the phone already trusts,
writes the config, udev rule and unit, checks the phone is reachable, and
enables the service.

## Use

```bash
adb-wan status                       # read-only report
adb-wan check                        # WAN health: exit 0 up, 2 down
sudo adb-wan test-usb-toggle         # hardware test: USB Ethernet off, then on, recorded
sudo adb-wan test-usb-enable         # hardware test: switch tethering off first; waits for ADB, enables rndis
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
