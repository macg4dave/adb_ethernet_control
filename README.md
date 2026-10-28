# ADB Ethernet Control

Recover the RHEL host's main WAN, supplied by a USB-tethered Android phone,
after a host restart leaves USB Ethernet enabled on the phone but unusable on
the host.

The boot recovery script waits for configured host services, checks WAN access
through the phone's interface, and, if needed, cycles USB networking off and
back on once. It then checks whether connectivity returned. A healthy WAN is
left alone. Implementation is Bash, ADB, and systemd.

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

Boot recovery is supplied as system units under `systemd/system/` and is not
installed or enabled automatically. The optional `systemd/user/` timer only
prints diagnostic reports. **USB cycling is disabled by default:** both live
HONOR tests required manual tethering restoration. Read-only checks work,
but automatic boot recovery is not ready to deploy.

See `AGENTS.md` and `CLAUDE.md` for shared development instructions. Checks
cover ShellCheck, mock-device regression tests, and systemd unit validation;
they do not reset a real phone or alter host networking. No Rust toolchain is
needed for the current version.
