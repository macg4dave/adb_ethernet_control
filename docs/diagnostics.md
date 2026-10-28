# Read-only diagnostics

Inspect a USB-connected Android phone's USB functions, network addresses,
routes, and tethering diagnostics from a Red Hat Enterprise Linux host.
The initial implementation is Bash; Rust is reserved for future features
that benefit from it. See [project purpose](project.md) and
[boot recovery](boot-recovery.md) for the WAN recovery behavior.

## Requirements

- Bash 4+, GNU coreutils (`timeout`), and Android SDK Platform Tools (`adb`).
- `iproute` for host network addresses; NetworkManager's `nmcli` is optional.
- For development: `shellcheck` and `systemd-analyze`.

Use your approved RHEL repositories for Bash, coreutils, iproute, NetworkManager,
and ShellCheck (package availability varies by enabled repositories). Install
ADB from your approved package source or the official
[Android SDK Platform Tools](https://developer.android.com/tools/releases/platform-tools).
No Rust toolchain is needed for this version.

Enable Developer options and USB debugging on the phone, connect a data-capable
USB cable, unlock it, and accept the computer's debugging authorization prompt.
ADB needs access to the USB device as your normal user. If access is denied,
check your distribution's Android USB/udev setup; do not run this tool as root
or disable SELinux to work around permissions.

## Run from the checkout

```bash
./bin/adb-ethernet-control devices
./bin/adb-ethernet-control status
./bin/adb-ethernet-control --serial YOUR_USB_SERIAL status
./scripts/check
```

`status` selects the sole authorized USB device or requires `--serial` when
several are available. TCP/IP transports are excluded. `devices` lists all
ADB transports so authorization problems are visible. Each ADB invocation has
a 15-second timeout; set `ADB_TIMEOUT` to another positive number of seconds.
Set `ADB_BIN` to an alternative ADB executable if needed.

The report includes requested/active USB functions, the USB service dump,
phone addresses and IPv4/IPv6 routes, the tethering service dump, and host
network devices. Functions such as `rndis` or `ncm` can indicate USB networking;
they alone do not establish working connectivity. Host interfaces are listed
for inspection; the tool does not yet correlate them to the selected phone.
Android vendor/version differences can restrict or omit diagnostics. Empty
properties and service messages are shown verbatim; a successful command does
not guarantee the service supplied useful data.

Exit codes: `0` means commands completed, `1` means invalid input or device
selection failure, and `2` means a report was produced with failed probes.
The tool does not enable tethering or change network settings. ADB may start
its local server as part of inspection.

## Troubleshooting USB discovery

If `lsusb` sees the phone but `adb devices -l` is empty, first check the USB
device permissions and whether an ADB interface is exposed:

```bash
lsusb
lsusb -t
lsusb -v -d 339b:108a
getfacl /dev/bus/usb/007/005
```

The vendor/product and bus/device values above are examples from an HONOR
ALI-NX1; use the current values from `lsusb`. Bus/device numbers can change
after reconnecting. This phone exposed `rndis_adb` with an `ADB Interface`,
and the user's USB ACL already allowed access, but an existing native-backend
ADB server listed no devices. Restarting with libusb made the phone visible:

```bash
adb kill-server
ADB_LIBUSB=1 adb start-server
adb devices -l
```

Restarting the ADB server interrupts other ADB clients. It does not require
changing host Ethernet settings. `ADB_LIBUSB` is selected when the server
starts; setting it on a client cannot change an already-running server.
Recent Platform Tools also provide `adb server-status` to show the backend.

If the device appears as `unauthorized`, unlock the phone and accept **Allow
USB debugging?** for this computer. Then rerun `devices` and `status`. If no
prompt appears, reconnect the USB cable with the phone unlocked and USB
debugging enabled. A visible ADB interface alone does not mean the computer
has been authorized.

## Safe user unit verification

Use `./scripts/check` for validation, or
`./scripts/verify-user-units /absolute/path/to/example.service` to verify user
units separately. The wrapper uses a private temporary runtime directory,
isolated user configuration and unit paths, disabled generators, and disconnected
bus addresses. It removes the temporary directory on success or failure.
On this host, verification can print `Failed to connect to system bus: No such
file or directory` because that bus is deliberately disconnected; the command's
exit status still determines whether unit validation passed.

On 28 October 2026, the original `scripts/check` invoked
`systemd-analyze --user verify` with the login session's `XDG_RUNTIME_DIR`.
An isolated reproduction on this host's systemd 257 build showed verification
replacing `$XDG_RUNTIME_DIR/systemd/private` with its own socket and leaving it
without a listener on exit. Terminal shell launches then reported
`Failed to connect to user scope bus via local transport: Connection refused`.
The user restored access by restarting `user@1000.service`. That restart can
terminate desktop applications and shells; it is not part of validation or the
fix. The regression test checks that verification preserves a caller's private
socket stand-in and cleans up its isolated runtime on success and failure.

## Optional systemd user timer

Install explicitly after checking the CLI works:

```bash
install -Dm755 bin/adb-ethernet-control "$HOME/.local/bin/adb-ethernet-control"
install -Dm644 systemd/user/adb-ethernet-control.service "$HOME/.config/systemd/user/adb-ethernet-control.service"
install -Dm644 systemd/user/adb-ethernet-control.timer "$HOME/.config/systemd/user/adb-ethernet-control.timer"
systemctl --user daemon-reload
systemctl --user enable --now adb-ethernet-control.timer
journalctl --user -u adb-ethernet-control.service
```

The timer runs a report approximately every minute after the preceding run
finishes. It operates while the user manager is running. With no authorized
phone the service fails with a diagnostic and the timer retries later.
Multiple phones require a service override to add `--serial YOUR_USB_SERIAL`
to `ExecStart`. For an ADB installation outside the service PATH, create
`~/.config/adb-ethernet-control/environment` with an absolute path such as
`ADB_BIN=/opt/platform-tools/adb`. This is a systemd environment file, not a
shell script. Do not put `export` in it.
If this host requires the libusb workaround above, add `ADB_LIBUSB=1` on a
separate line in that environment file as well. An already-running server
must first be restarted with that setting.
If using the dedicated boot-recovery ADB server, also add
`ADB_SERVER_SOCKET=tcp:localhost:5038` so the reporting service uses the server
that already owns the phone interface.

Reports can contain device identifiers and network details. Timer output is
stored in your user journal; consider this before enabling recurring reports.
Stop recurring inspection with:

```bash
systemctl --user disable --now adb-ethernet-control.timer
```

## Development

Run examples from the repository root. `AGENTS.md` contains shared project
rules and Codex guidance; `CLAUDE.md`
directs Claude to those same rules. Run `./scripts/check` for Bash syntax,
ShellCheck, mock-device regression tests, and systemd unit validation.
Hardware verification still requires a phone: check authorized/unauthorized
USB debugging, tethering enabled/disabled, and host interface visibility.
Do not commit captured reports or device credentials.
