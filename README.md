# ADB Ethernet Control

Recover the RHEL host's main WAN, supplied by a USB-tethered Android phone,
after a host restart leaves USB Ethernet enabled on the phone but unusable on
the host.

Boot recovery also handles USB Ethernet being off. Recovery uses the dedicated
user-owned ADB server without auto-starting a competing root-owned daemon;
reinstall with the command below to apply the startup race fix.

The boot service waits for host startup and checks WAN through the phone's
interface. If USB Ethernet is off, it requests enablement. If Ethernet is on
but WAN is down, the guarded cycle branch switches it off/on once with a
one-second pause. After enabling, it waits 20 seconds for tethering and up to
45 seconds for host IPv4 addressing. WAN checks ping `8.8.8.8` through the
phone interface. It checks WAN, waits 10 seconds, checks again, and logs one
final error if still down. A healthy WAN is untouched and needs no ADB.

- [Project purpose, findings, and acceptance criteria](docs/project.md)
- [Boot recovery configuration, installation, and validation](docs/boot-recovery.md)
- [Read-only diagnostics and USB troubleshooting](docs/diagnostics.md)
- [Local tests that keep running without the WAN or AI](docs/offline-testing.md)

```bash
./bin/adb-ethernet-control devices
./bin/adb-ethernet-control status
PHONE_SERIAL=YOUR_USB_SERIAL WAN_INTERFACE=enp48s0f4u2 \
  ./bin/adb-wan-recover --check
./scripts/check
```

Install, check, and enable boot recovery from a local terminal:

```bash
./install.sh
```

The guided installer finds defaults for your USB serial, WAN interface, ADB
path, and readiness services. Each answer is prefilled and editable, with tips
for finding it manually. It reuses readable settings from an existing install;
environment settings take priority. It shows the settings before installation. Run it
as your normal user; it requests sudo when needed. Use `./install.sh --dry-run`
to preview or `./install.sh --help` for unattended usage.

This requires sudo, verifies the unit, runs it, and enables it only after a
successful run. It does not reboot the host. The optional user timer only
prints diagnostic reports. **USB cycling is disabled by default:** both live
HONOR tests required manual tethering restoration. Read-only checks work,
and the earlier host-side reconnect passed its supervised outage test. The
attended reboot required manual tethering restoration. The new enable-only
branch and independent restore timer have mock coverage; phone-side behavior
still needs hardware validation. Cycling stays disabled until independent
restoration is verified. See the boot guide for the exact limitation.

One later attended MTP/RNDIS test passed without manual intervention: USB off/on,
ADB returning, and final WAN connectivity were observed. The fallback timer did
not need to fire, so independent restoration remains unverified.

To explicitly test only whether USB Ethernet switches off/on, from a local
terminal:

```bash
WAN_INTERFACE=enp48s0f4u2 ./scripts/start-local-check --test-usb-toggle
```

This manual flag permits one supervised cycle with a one-second pause while
automatic cycling remains disabled. It requires a healthy WAN baseline and
observed USB off/on states to pass; subsequent WAN results are informational.
It needs sudo and records results in `/var/log/adb-wan-usb-test/`. Both earlier
cycles needed manual repair, so remain able to re-enable tethering on the phone.

See `AGENTS.md` and `CLAUDE.md` for shared development instructions. Checks
cover ShellCheck, mock-device regression tests, and systemd unit validation;
they do not reset a real phone or alter host networking. No Rust toolchain is
needed for the current version.
User-unit verification uses a disposable runtime directory so it cannot replace
the desktop user manager's private socket.
