# ADB Ethernet Control

Recover the RHEL host's main WAN, supplied by a USB-tethered Android phone,
after a host restart leaves USB Ethernet enabled on the phone but unusable on
the host.

The boot service waits for host startup and checks WAN through the phone's
interface. If USB Ethernet is off, it requests enablement. If Ethernet is on
but WAN is down, the guarded cycle branch switches it off/on once with a
one-second pause. It checks WAN, waits 10 seconds, checks again, and logs one
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
PHONE_SERIAL=YOUR_USB_SERIAL WAN_INTERFACE=enp48s0f4u2 \
  ./scripts/install-boot-service
```

This requires sudo, verifies the unit, runs it, and enables it only after a
successful run. It does not reboot the host. The optional user timer only
prints diagnostic reports. **USB cycling is disabled by default:** both live
HONOR tests required manual tethering restoration. Read-only checks work,
and the earlier host-side reconnect passed its supervised outage test. The
attended reboot required manual tethering restoration. The new enable-only
branch and independent restore timer have mock coverage; phone-side behavior
still needs hardware validation. Cycling stays disabled until independent
restoration is verified. See the boot guide for the exact limitation.

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
