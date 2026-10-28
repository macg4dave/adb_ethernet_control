# Boot USB Ethernet recovery

After startup, the boot service checks WAN through the phone interface. A
healthy WAN is untouched and requires no ADB access. When WAN is down, it
selects the configured authorized USB phone and reads its actual USB state
(`sys.usb.state`). The state is judged as a set of functions: `rndis` or `ncm`
means on; `adb` without either means off, including vendor sets such as
HONOR's boot-time `hisuite,mtp,mass_storage,adb`. The read is retried for up
to `PHONE_TIMEOUT`, since states can be empty during enumeration.


- USB Ethernet off: request the configured RNDIS or NCM function once.
- USB Ethernet on but WAN down: perform one off/on cycle, with a **one-second
  pause between off and on**, only if independent restoration has been verified
  and cycling explicitly enabled.
- Check WAN after the recovery decision, wait **10 seconds**, then check WAN
  again. If the final check fails, log one error including whether USB Ethernet
  is on, off, or unknown, and exit. Never repeat the cycle.

After requesting enablement, wait `POST_ENABLE_DELAY` (20 seconds by default)
for tethering, then up to `CONNECTION_TIMEOUT` (45 seconds) for a global IPv4
address on the host phone interface before the WAN checks. Failure to acquire
an address still produces a final failed result without another USB cycle.

The initial WAN check follows readiness and startup settling. After recovery,
the second WAN check determines the final outcome even if the first succeeded.
Probe execution time is additional to the one-second and ten-second pauses.
NetworkManager autoconnect remains responsible for DHCP and routing. Boot
recovery does not deactivate host profiles or change LAN/firewall settings.

## Installation

From a local terminal in the checkout, with the phone already authorized:

```bash
./install.sh
```

The guided installer finds defaults for your USB serial, WAN interface, ADB
path, and readiness services. Each answer is prefilled: edit it using the arrow
keys and Backspace, or press Enter to accept. Each question includes a tip for
finding the information manually. It shows the settings before installation. Run it
as your normal user; it requests sudo when needed. Use `./install.sh --dry-run`
to preview or `./install.sh --help` for unattended usage.

Environment settings take priority, followed by readable values from
`/etc/adb-ethernet-control/boot.env`, followed by read-only detection. ADB is
located on `PATH`. Phone discovery queries existing loopback ADB servers on
ports 5038 and 5037 with bounded timeouts; it never starts a server. Only a
single authorized USB phone becomes a suggested serial. Absent, unauthorized,
offline, or ambiguous phones require a manual answer.

Interface discovery matches USB network devices to the selected phone serial.
If no serial is exposed, a sole RNDIS/NCM interface can be suggested. It never
chooses an interface with a different phone serial. If the interface is absent
because tethering is off, reuse the saved setting or enter the interface from
the phone's NetworkManager profile. Readiness defaults to NetworkManager and
adds `pc101-technitium.service` only when systemd reports it installed.
Detection does not enable USB networking or alter host networking.
`--yes` accepts supplied or unambiguous detected settings; missing required
answers still stop installation.

The installer requires sudo. It installs root-owned boot executables, a
systemd-owned ADB server running as the invoking user, and headless USB access
rules for this HONOR phone. Rules take effect on the next USB enumeration;
the installer does not reset USB devices. The ADB server uses that user's
existing keys, libusb, and port 5038. It is started only if WAN is unhealthy.
A healthy immediate check does not validate ADB access. No phone identifiers
or keys are written into Git.

Managed recovery clients connect explicitly to `127.0.0.1:5038`, preventing
ADB from spawning a competing root-owned server during startup. Phone discovery
retries within `PHONE_TIMEOUT` while the user-owned server becomes ready. A
server failure stops recovery without USB changes; the server does not restart
in a loop. Reinstallation stops both old units before running the updated
recovery, clearing any legacy ADB daemon owned by the recovery unit.

The installer writes `/etc/adb-ethernet-control/boot.env`, verifies the unit,
runs the boot service, and enables it only after a successful run. It does
not reboot. Review the readiness list to include your LAN DHCP/DNS service;
the lower-level `scripts/install-boot-service` retains its host-specific default
`NetworkManager.service pc101-technitium.service` when called directly without
`READY_UNITS`. Reinstalling replaces the generated configuration and resets
`USB_CYCLE_ENABLED=0`.

## Restoration limitation

**Automatic cycling remains disabled by default.** Both earlier live USB
cycles lost control and required manual tethering restoration. The attended
reboot test also required manual tethering restoration: the phone was present
on USB but exposed no network interface until the user enabled tethering.
The old host-side NetworkManager reconnect could not fix that condition. Its
passing record reflected the user's manual repair during the initial wait.

The new enable-only branch can request USB networking when it is off. Whether
`svc usb setFunctions` also starts this vendor's tethering/DHCP service still
requires hardware validation; a successful command alone is insufficient.

Before a cycle, the new coordinator arms a separate systemd restore timer.
The off step requests MTP rather than the earlier charging-only mode; this is
intended to retain debugging. One attended manual test verified the MTP/RNDIS
cycle and WAN restoration without manual intervention; see the
[hardware record](offline-testing.md#explicit-usb-toggle-test).
After one second, the coordinator waits for the pinned USB ADB transport and
requests the original networking function. It never uses ADB root. The
independent timer survives coordinator termination and runs the same script
in enable-only `--restore rndis` or `--restore ncm` mode after 30 seconds.
The timer is canceled after successful final WAN and USB-state checks. On
failure it remains armed. This fallback cannot restore a phone that stops
exposing ADB altogether. Keep `USB_CYCLE_ENABLED=0` until an independent
restore path has been verified in an explicitly requested, attended hardware
test. An unhealthy on-state with cycling disabled still receives the two WAN
checks and one final error; it is not silently treated as repaired.

The explicit manual `scripts/start-local-check --test-usb-toggle` flag can
exercise one USB off/on cycle without enabling the automatic branch. It needs
a healthy starting WAN, system-manager supervision, and a restore timer. Its
pass/fail concerns observed USB states; WAN results are informational. See
[manual testing](offline-testing.md#explicit-usb-toggle-test) for the command
and hardware limitations.

## Bounds and records

Defaults wait up to 120 seconds for readiness services, settle for 15 seconds,
and wait up to 60 seconds for the configured USB phone when recovery is needed.
Absent, unauthorized, offline, non-USB, ambiguous, or unexpected USB states
produce a control failure without a USB toggle. Other phones can coexist;
commands use only the configured phone's current USB transport ID, reselected
after enumeration. Malformed, empty, or ADB-less states are not assumed to mean
Ethernet is off; the error log includes the validated raw state.

WAN health requires a global IPv4 address and a successful IPv4 ping to
`8.8.8.8`, bound to `WAN_INTERFACE` so another host uplink cannot mask a broken
phone WAN. Each ping has a five-second timeout (`PROBE_TIMEOUT`). This check
does not require DNS, HTTPS or correct TLS time. Networks that block ICMP to
that address will fail this check even if other Internet access works.
The legacy manual CLI continues to use its configured HTTPS probes; these
boot settings apply to `adb-wan-boot`.

External operations have timeouts. The local runner limits the boot command
to 600 seconds and records diagnostics after failure. The system unit has a
15-minute final limit. Restore workers have their own 100-second limit.
No automatic service restart is configured.

After reboot, inspect the local journal and persistent record:

```bash
systemctl status adb-wan-recover.service --no-pager
journalctl -b -u adb-wan-recover.service
ls -lt /var/log/adb-wan-boot/
cat /var/log/adb-wan-boot/run-XXXXXXXX/result
cat /var/log/adb-wan-boot/run-XXXXXXXX/output.log
journalctl -b -u 'adb-wan-usb-restore-*'
```

Replace the directory placeholder with the current run. Records are root-owned
and readable by the installer's primary group. Result status is `running`,
`passed`, or `failed`; a leftover `running` record is incomplete. Manual
intervention during a run must be considered when interpreting success.
Exit codes are 0 for healthy/restored, 1 for configuration/readiness/control
failure, and 2 for WAN unavailable on the final check. The oneshot stays active
after success and does not repeat automatically. Disable future runs with:

```bash
sudo systemctl disable adb-wan-recover.service
```

`./scripts/check` uses fake devices and never changes real networking. Hardware
validation must still cover enable-only recovery with the new addressing wait
and interface-bound ping, actual timer restoration with
the coordinator stopped, and an attended reboot. One manual cycle has verified
ADB returning, addressing, and WAN connectivity after off/on. Live tests must
run under the systemd system manager
with local records; see [offline testing](offline-testing.md).
