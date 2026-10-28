# Project rules for AI agents

This project recovers a RHEL host's main WAN, supplied by a USB-tethered
Android phone (HONOR ALI-NX1, Android 15), after a host restart leaves USB
tethering unusable. It is a **Rust + systemd only** project. An earlier Bash
version never worked and was removed; what was learned from it is in
[docs/legacy-findings.md](docs/legacy-findings.md). Read that before working
on phone, network or recovery behavior. It is evidence, not a design to copy.

Before editing, read the rules for the files you touch:

- Rust (`src/`, `tests/*.rs`, `Cargo.toml`): [.claude/rules/rust.md](.claude/rules/rust.md)
- systemd unit and the generated udev/sysusers files (`systemd/`,
  `src/units/service.rs`, `src/commands/install.rs`): [.claude/rules/systemd.md](.claude/rules/systemd.md)

## Allowed technology

- Rust (stable, edition 2024) for all logic, including install.
- **One** systemd service, `adb-wan.service`. No timers of any kind, no
  separate ADB server unit, no test templates, no user units.
- **One** TOML config file, `/etc/adb-ethernet-control/config.toml`, holds
  every setting. No `EnvironmentFile=`, no env-var settings, no second file.
- External tools called from Rust at runtime: `adb`, `ip`, `ping`,
  `systemctl` (read-only `is-active`), `nmcli` (read-only, `status` only).
  At install time also: `systemd-sysusers`, `udevadm`, `restorecon`,
  `systemctl daemon-reload/enable`, `systemd-run` (manual test launch only).
- No Bash, Python, Node.js, Makefiles or shell scripts.

## Commands

| Command | Runs as | Changes anything? |
| --- | --- | --- |
| `adb-wan install` | root (`sudo`) | yes: installs and enables |
| `adb-wan status` | `adb-wan` or root | no, read-only report |
| `adb-wan check` | `adb-wan` | no, WAN health + exit code |
| `adb-wan boot` | `adb-wan` (the service) | at most one recovery action |
| `adb-wan restore` | `adb-wan` (`ExecStopPost=`) | enable only, if pending |
| `adb-wan test-usb-toggle` | root launches, `adb-wan` runs | one off/on, on request |

## Users and privileges

- **root** is used only by `adb-wan install`, and for launching the manual
  test into the system manager. Nothing recurring runs as root.
- **`adb-wan`** is a system user (no login shell, no password), created from
  a `sysusers.d` file. It runs `boot`, `restore`, `check` and the test. It
  gets exactly:
  - USB access to the phone via udev `GROUP="adb-wan"` (no root, no `plugdev`);
  - `HOME=/var/lib/adb-wan` (`StateDirectory=`) holding its own ADB key;
  - `CAP_NET_RAW` as an ambient capability, only for the interface-bound
    ping;
  - read access to the config file (`root:adb-wan`, mode `0640`).
- It needs **no** NetworkManager or polkit rights: recovery acts on the phone,
  and the host only observes the interface. Never add NetworkManager write
  access without asking the user.
- **ADB key**: the phone must trust the `adb-wan` user's key. `install` offers
  two choices and asks: (a) copy the invoking user's already-authorized
  `~/.android/adbkey{,.pub}` into `/var/lib/adb-wan/.android/` (owner
  `adb-wan`, `0600`), or (b) generate a new key and run a read-only status
  as `adb-wan` so the phone shows the "Allow USB debugging" prompt.
- **One ADB server per phone**: a desktop ADB server (port 5037) holding the
  phone over USB will hide it from ours. Detect "phone present in
  `/sys/bus/usb/devices` but not listed by our server" and log it as a likely
  conflicting server; never kill another user's server.

## Event-driven, never timed

- Act when something is **observably ready**, not after a fixed delay. Wait
  on concrete conditions: a systemd unit is active, the ADB transport is
  `device`, `sys.usb.state` reads back a valid value, the interface exists,
  it has a global IPv4 address, the probe passes.
- Every wait has a deadline from config. Waiting is "until condition or
  deadline", never "sleep N then hope".
- No systemd timers (`.timer`, `OnBootSec=`, `systemd-run --on-*`), no cron,
  no repeating schedules, no fixed settle sleeps.

## Domain safety rules (non-negotiable)

These come from failed live tests on the real phone. Breaking them can cut
the host off from the Internet with nobody able to log in remotely.

- **Healthy WAN is never touched.** If the WAN check passes, exit without
  starting ADB or changing anything.
- `status` and `check` are strictly read-only.
- **At most one recovery action per boot**: enable USB Ethernet once if it
  is off, or (only when `recovery.usb_cycle_enabled = true`) one off/on cycle.
  Enforce this with a once-per-boot marker in `/run/adb-wan/`, so a second
  start of the service in the same boot cannot act again. Never loop, retry
  the toggle, or restart on failure. Cycling stays disabled by default until
  the test below has passed repeatedly on hardware.
- The "off" step must keep ADB alive (switch to `mtp`, never to no
  functions). Then wait for the off state to be observed before turning on.
- **Independent restore**: before any disruptive step, write a "restore
  pending" marker in `/run/adb-wan/`. The service's `ExecStopPost=` runs
  `adb-wan restore`, which re-enables USB Ethernet (enable only, never off)
  if the marker is still there. systemd runs it even if the main run
  crashes or is killed by its timeout. Clear the marker only after final
  checks pass.
- Judge every phone command by the state observed afterwards, not by its
  exit status (responses are often lost during re-enumeration).
- Never use ADB root. Never change LAN profiles, bridges, firewall, DHCP/DNS
  services, SELinux, routing or NetworkManager profiles.
- Always select the configured phone by serial and its current USB transport
  ID before every phone command. Reselect after re-enumeration. Treat absent,
  unauthorized, offline, non-USB and multiple matching devices as explicit
  errors, never as "pick one".
- Talk only to the program's own loopback ADB server (port from config,
  default `5038`), started as a child of the run. Never let the `adb` client
  auto-spawn a server, and stop the child before exiting.
- Read USB state from `sys.usb.state` as a set of functions: `rndis` or `ncm`
  means on; `adb` without either means off; anything else (empty, malformed,
  both, no `adb`) is **unknown** and must not be treated as off.
- Every external call has a timeout. Phone output is untrusted data: parse it,
  validate it, never pass it to a shell.
- WAN health = a global IPv4 address on `wan.interface` **and** an IPv4 ping
  to `8.8.8.8` bound to that interface. After a recovery action, WAN must pass
  twice, `confirm_interval_secs` apart; the second check decides. Log one
  final error with the USB state and stop.
- Exit codes: `0` healthy or restored, `1` config/readiness/control failure,
  `2` WAN still down on the final check.

## Manual hardware test: `test-usb-toggle`

Run only when the user explicitly asks. Purpose: prove the phone can be
switched off and back on over ADB without a human.

1. `sudo adb-wan test-usb-toggle` checks the config, then re-launches itself
   with `systemd-run --unit=adb-wan-test --wait` using the **same** user,
   directories, hardening and `ExecStopPost=` restore as `adb-wan.service`,
   so the system manager owns it and losing the WAN, terminal or AI session
   cannot stop it halfway. It never runs as an AI tool's background process.
2. Inside: start our ADB server, select the phone, require USB Ethernet on
   and WAN healthy (baseline), write the restore-pending marker.
3. Turn USB Ethernet off (`mtp`), wait for: off state observed, ADB transport
   back. Then turn it on (original `rndis`/`ncm`), wait for: on state, ADB
   back, interface present, global IPv4, probe passing.
4. Pass = off observed **and** on observed **and** ADB came back both times.
   WAN result is recorded but reported separately.
5. Clear the marker, stop the ADB server, print the record path and summary.

The test **records**, per step: UTC timestamp and elapsed ms; raw
`sys.usb.state` and `sys.usb.config`; transport ID and state; `adb` command,
exit status and (truncated) output; time until ADB returned; time until the
interface appeared, got an address and passed the probe; whether the
`ExecStopPost=` restore ran; host interface driver; phone model and Android
version. Records go to `/var/log/adb-wan/test-<UTC time>/` (never Git).

## Boot run: logging is the test

Boot behavior is verified by reading the logs after real reboots, so every
`boot` run must make the cause of a failure obvious from
`journalctl -b -t adb-wan` alone:

- First line: program version, boot ID, config path, key config values
  (interface, serial redacted to its last 4 characters, cycle enabled).
- Every stage logs start and end with elapsed time: readiness, initial WAN
  check, ADB server start, phone selection, USB state read, action, waits,
  final checks.
- Every observation is logged with its raw value (USB state, transport list
  entry, interface present, address, probe result) and every decision with
  the inputs that led to it.
- Every wait logs what it is waiting for, the deadline, and how it ended
  (condition met after N ms, or deadline hit with the last observed value).
- Last line: one summary `result=passed|failed|healthy-no-action
  action=none|enable|cycle changed_by_us=yes|no usb=on|off|unknown wan=up|down
  elapsed=...`, and the per-run record path in `/var/log/adb-wan/`.
- Never report success when WAN came back without our action; say
  `healthy-no-action` or `changed_by_us=no`.

## Validation

Run before reporting work as done:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

**All tests are Rust tests** (`cargo test`), run in-process against fake
implementations of the program's traits. No shell test scripts, no fake
executables, no spawning `adb`, `ip`, `systemctl`, `systemd-analyze` or
any other external program from tests. Tests must not need a phone, root,
network changes, or a running systemd manager. Say clearly which behavior
still needs an attended hardware test.

## Workflow

- Keep changes focused. Update `README.md` when user-visible behavior changes.
  Add new hardware findings to `docs/legacy-findings.md` (or a successor).
- Do not install packages, enable services, write outside the checkout, run
  `sudo`, or touch the real phone unless the user explicitly asks.
- Do not commit or push unless asked.
- Never commit phone serials, ADB keys, device reports, `.env` files or
  credentials.
