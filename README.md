# ADB Ethernet Control (`adb-wan`)

Keeps a Linux host online when its internet comes from an **Android phone
over USB tethering**.

After the host restarts, the phone often comes back with USB tethering
switched off, and the host has no internet. `adb-wan` runs once at every
boot, checks the WAN and, if it is down, uses ADB to switch USB tethering
back on. If the WAN is already working it does nothing at all.

Tested with an HONOR ALI-NX1 (Android 15) on RHEL 10.

---

## Contents

- [How it works](#how-it-works)
- [Requirements](#requirements)
- [1. Set up the phone](#1-set-up-the-phone)
- [2. Install](#2-install)
- [3. Check it works](#3-check-it-works)
- [Everyday use](#everyday-use)
- [Settings](#settings)
- [Logs and records](#logs-and-records)
- [Troubleshooting](#troubleshooting)
- [Uninstall](#uninstall)
- [Development](#development)

---

## How it works

At every boot, `adb-wan.service` runs once:

1. Waits for NetworkManager (and any services you list) to start.
2. Checks the WAN: the phone's interface has an IPv4 address and a ping to
   `8.8.8.8` through that interface works.
   **If it works, it stops here.** Nothing is changed and ADB is not started.
3. Otherwise it starts its own private ADB server and asks the phone for
   its USB mode:
   - **USB tethering off** → switches it on once (`svc usb setFunctions rndis`).
   - **USB tethering on, but no internet** → waits for NetworkManager to
     finish. Optionally (off by default) it switches tethering off and on
     once.
4. Waits for the WAN to work, writes one summary line to the journal and
   stops.

It acts **at most once per boot**, never loops, and never touches your LAN,
firewall or NetworkManager profiles. There are no timers: every step waits
for something it can observe, with a safety limit.

---

## Requirements

- Linux with **systemd** and **NetworkManager** (tested on RHEL 10).
- **adb** (Android platform tools), installed outside `/home`, for example
  `/usr/local/bin/adb`.
- **Rust** (stable) to build: `cargo`.
- An Android phone connected by a USB cable that carries data, with USB
  tethering as the host's internet connection.

---

## 1. Set up the phone

These phone settings are **required**. Without them the phone switches ADB
off together with tethering, and nothing can switch tethering back on.

1. Enable **Developer options**: *Settings → About phone →* tap
   **Build number** 7 times.
2. In *Settings → System → Developer options*, switch **on**:
   - **USB debugging**
   - **Allow ADB debugging in charge only mode**
3. **Unplug the USB cable and plug it back in**, so the phone applies the
   setting.
4. When the phone asks **"Allow USB debugging?"**, tick
   **Always allow from this computer** and tap **Allow**.
5. Switch on **USB tethering** so the host is online.

> You do **not** need HiSuite or HDB.

---

## 2. Install

Get the code and build it:

```bash
git clone https://github.com/macg4dave/adb_ethernet_control.git adb_ethernet_control
cd adb_ethernet_control
cargo build --release
```

Preview what the installer would do (changes nothing, no sudo needed):

```bash
./target/release/adb-wan install --dry-run
```

Install. It finds your phone's serial and network interface by itself and
asks you to confirm each answer (press **Enter** to accept):

```bash
sudo ./target/release/adb-wan install
```

If your LAN has its own DHCP/DNS service that should be running first, name
it (repeat `--ready-unit` for each service):

```bash
sudo ./target/release/adb-wan install \
  --ready-unit NetworkManager.service \
  --ready-unit pc101-technitium.service
```

### What the installer does

| Step | Result |
| --- | --- |
| Creates a system user | `adb-wan` (no login), from `/etc/sysusers.d/adb-wan.conf` |
| Installs the program | `/usr/local/bin/adb-wan` |
| Writes the settings | `/etc/adb-ethernet-control/config.toml` |
| Gives the user USB access to *this* phone only | `/etc/udev/rules.d/70-adb-wan.rules` |
| Prepares the ADB key | `/var/lib/adb-wan/.android/` (offers to copy your already-trusted key) |
| Writes the service | `/etc/systemd/system/adb-wan.service` and `adb-wan.service.d/10-readiness.conf` |
| Checks the phone is reachable | as the `adb-wan` user, before enabling anything |
| Enables and runs the service once | with a healthy WAN this changes nothing |

### Installer options

| Option | Meaning |
| --- | --- |
| `--dry-run` | Show the steps, change nothing |
| `--yes` | Accept detected answers without asking |
| `--serial SERIAL` | Phone USB serial (`adb devices -l`, the line with `usb:`) |
| `--interface NAME` | Host interface of the phone, e.g. `enp48s0f4u2` |
| `--adb PATH` | Path to `adb` (must not be in `/home`) |
| `--ready-unit UNIT` | Service to start after; repeatable |
| `--key copy` / `--key new` | Copy your trusted ADB key, or make a new one (accept the prompt on the phone) |

Run the installer again at any time to update the program or change these
answers.

---

## 3. Check it works

Status report (read-only):

```bash
sudo adb-wan status
```

Hardware test: switch tethering **off** on the phone when asked, and watch
`adb-wan` switch it back on:

```bash
sudo adb-wan test-usb-enable
```

> While tethering is off the host is offline. The test runs under systemd,
> so it finishes even if your terminal or remote session drops.

Then the real test: **reboot** the host and read what happened:

```bash
journalctl -b -t adb-wan
```

---

## Everyday use

You normally do nothing: the service runs by itself at every boot.

| Task | Command |
| --- | --- |
| What did the last boot do? | `journalctl -b -t adb-wan` |
| Just the result line | `journalctl -b -t adb-wan \| grep 'result='` |
| Is the WAN healthy now? (exit 0 = up, 2 = down) | `sudo adb-wan check; echo $?` |
| Full report: interface, WAN, USB devices, phone | `sudo adb-wan status` |
| Service state | `systemctl status adb-wan.service` |
| Run the boot check again now | `sudo systemctl restart adb-wan.service` |
| Watch live | `journalctl -f -t adb-wan` |
| Hardware test: tethering off → on | `sudo adb-wan test-usb-enable` |
| Hardware test: one USB off/on cycle | `sudo adb-wan test-usb-toggle` |

> `systemctl restart` will not act a second time in the same boot: recovery
> runs at most once per boot by design. The hardware tests can be repeated.

### Reading the result line

Every boot run ends with one line, for example:

```text
result=passed action=enable changed_by_us=yes usb=on wan=up elapsed_s=3
```

| `result=` | Meaning |
| --- | --- |
| `healthy-no-action` | WAN was working; nothing was changed |
| `passed` | `adb-wan` changed something and the WAN works now |
| `failed` | WAN still down after its one attempt (exit code 2) |
| `error` | Could not reach the phone or another problem; the line before says why (exit code 1) |

`changed_by_us=no` with a working WAN means it came back without `adb-wan`
(for example you fixed it by hand).

---

## Settings

All settings are in one file:

```bash
sudoedit /etc/adb-ethernet-control/config.toml
```

Changes apply at the next run. After changing `[readiness] units`, run the
installer again so the service ordering is updated. The installer keeps an
existing config if your answers are unchanged.

```toml
[phone]
serial = "YOUR_PHONE_SERIAL"     # adb devices -l, the line with usb:
usb_function = "rndis"           # "rndis" or "ncm"

[wan]
interface = "enp48s0f4u2"        # host interface of the phone
probe_host = "8.8.8.8"           # ping target, through that interface

[adb]
binary = "/usr/local/bin/adb"
server_port = 5038               # adb-wan's own server; not the desktop 5037
libusb = true
home = "/var/lib/adb-wan"        # holds the ADB key the phone trusts

[readiness]
units = ["NetworkManager.service"]   # add your LAN DHCP/DNS service

[recovery]
usb_cycle_enabled = false        # off/on cycle when tethering is on but broken
off_function = "mtp"             # "off" mode that keeps ADB alive

[paths]
ip = "/usr/sbin/ip"
ping = "/usr/bin/ping"
systemctl = "/usr/bin/systemctl"
nmcli = "/usr/bin/nmcli"
log_dir = "/var/log/adb-wan"
runtime_dir = "/run/adb-wan"
```

| Setting | When to change it |
| --- | --- |
| `phone.serial`, `wan.interface` | New phone or different USB port/interface (or rerun the installer) |
| `phone.usb_function` | Your phone tethers with NCM instead of RNDIS |
| `readiness.units` | A LAN service should be running before the check |
| `recovery.usb_cycle_enabled` | Set `true` only after `sudo adb-wan test-usb-toggle` has passed several times |

There are no timing settings. Unknown or misspelled keys are rejected with a
clear error, so a typo never silently changes behavior.

---

## Logs and records

Everything is in the system journal under the tag `adb-wan`:

```bash
journalctl -b -t adb-wan            # this boot
journalctl -t adb-wan --since today
journalctl -b -1 -t adb-wan         # previous boot
```

Each run also leaves a record folder:

```bash
sudo ls -lt /var/log/adb-wan/
sudo sh -c 'tail -n 4 /var/log/adb-wan/boot-*/result'
```

| Folder | Written by |
| --- | --- |
| `boot-<time>/` | the boot service |
| `test-enable-<time>/` | `test-usb-enable` |
| `test-toggle-<time>/` | `test-usb-toggle` |
| `restore-<time>/` | the safety restore after an interrupted off/on |

Each folder has `result` (status and summary) and `events` (the full log).
A `status=running` that never changed means the run was killed.

The log shows every USB device by port, its IDs and whether it offers ADB,
every time that changes, so you can see exactly what the phone did.

---

## Troubleshooting

**"another ADB server is listening on 127.0.0.1:5037"**
A desktop `adb` grabbed the phone. Only one ADB server can hold it. Stop it:

```bash
adb kill-server
```

**"phone is on USB but offers no ADB interface in this USB mode"**
The phone switched ADB off. Turn on **Allow ADB debugging in charge only
mode** ([phone setup](#1-set-up-the-phone)), then unplug and replug the cable.

**"phone is unauthorized"**
Unlock the phone and accept **Allow USB debugging** with **Always allow**.
If no prompt appears, unplug and replug the cable with the phone unlocked.

**"no USB device with the phone's serial is attached"**
Check the cable carries data and the phone is plugged in:

```bash
sudo adb-wan status      # see the "USB devices" list
```

**The service shows `failed`**
That is the honest result of a run that could not fix the WAN. Read why:

```bash
journalctl -b -t adb-wan | tail -n 30
```

**Config error at start**
The message names the bad key. Fix it with `sudoedit`, or compare with
[`config/config.example.toml`](config/config.example.toml).

---

## Uninstall

```bash
sudo systemctl disable --now adb-wan.service
sudo rm -f /etc/systemd/system/adb-wan.service
sudo rm -rf /etc/systemd/system/adb-wan.service.d
sudo rm -f /etc/udev/rules.d/70-adb-wan.rules /etc/sysusers.d/adb-wan.conf
sudo rm -f /usr/local/bin/adb-wan
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
```

Optional, also remove settings, the service user's ADB key and the logs:

```bash
sudo rm -rf /etc/adb-ethernet-control /var/lib/adb-wan /var/log/adb-wan
sudo userdel adb-wan
```

---

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests are Rust only and run without a phone, root or network changes.

- Rules for contributors and AI agents: [AGENTS.md](AGENTS.md)
- What was learned on real hardware: [docs/legacy-findings.md](docs/legacy-findings.md)
