# Findings from the Bash implementation

The Bash version (removed 2026-10-28) never achieved its goal: no automatic
recovery was ever observed to work across a real reboot. This file keeps
only what was learned from it. Treat techniques here as evidence, not as a
design to copy.

## Hardware and host facts (observed 2026-10-28)

- Phone: HONOR ALI-NX1, Android 15, USB vendor ID `339b`. The product ID
  changes as USB functions change, so match on vendor + ADB interface
  (`ID_USB_INTERFACES` contains `:ff4201:`), not on product ID.
- Host: RHEL 10, systemd 257. NetworkManager profile `honor-usb-wan` on
  `enp48s0f4u2`, autoconnect and IPv4 DHCP. Host driver `rndis_host`
  (`cdc_ncm` for NCM). LAN DNS/DHCP is `pc101-technitium.service`.
- `NetworkManager-wait-online.service` failed during the inspected boot, so
  `network-online.target` cannot be trusted to mean the phone WAN is up.
- `sys.usb.state` values seen: `rndis,adb` (tethering on), and at boot
  vendor sets such as `hisuite,mtp,mass_storage,adb` and `mtp,adb,hdb`
  (tethering off). The value can be empty during enumeration. Strip `\r`.
- `svc usb setFunctions <fn>` and `svc usb getFunctions` work.
  `cmd usb` and `cmd tethering` do not implement shell commands.
  Never include `adb` in the `setFunctions` mask; `svc` manages it.
- After `svc usb setFunctions` both command responses were reported as
  failed or lost, yet the state change happened. **Judge success by the
  observed state afterwards, never by the command's exit status.**
- Tethering takes 10–15 s after USB Ethernet is enabled before WAN works.
  Waiting for an address and a passing probe is what matters.
- ADB with the native USB backend listed no devices; restarting the server
  with `ADB_LIBUSB=1` fixed discovery. The backend is chosen when the server
  starts, not per client.
- The ADB key the phone authorized belongs to whichever `HOME` the server
  ran with. A new service user needs its key authorized on the phone once.
- Launching binaries from the home checkout under systemd failed with
  `203/EXEC` (SELinux label). Install binaries under `/usr/local/bin` and
  run `restorecon` on them.
- `ip` lives in `/usr/sbin` on RHEL; give units a full `PATH` or use absolute
  program paths.

## What failed, and the lesson

1. **First live USB off/on**: switching USB networking off also removed ADB
   (charging-only mode), so the host could not send the "on" command. User
   restored tethering by hand.
   Lesson: the "off" step must keep ADB alive. Switching to `mtp` instead
   of empty functions kept ADB on this phone.
2. **Second live cycle**: a detached `nohup setsid sh -c` worker on the phone
   was supposed to do off/wait/on itself. Its log stopped during the off
   command; manual restore needed again.
   Lesson: phone-side background shells are unreliable; do not use them.
3. **Attended reboot #1**: phone present on USB but exposed no network
   interface. The old host-side `nmcli connection up` cannot fix that. The
   run recorded `passed` only because the user enabled tethering by hand
   during the wait.
   Lesson: a "passed" record must say whether the program changed anything;
   a missing interface means the phone side is off.
4. **Attended reboot #2**: a root-owned ADB daemon was auto-spawned by the
   recovery client while the dedicated server failed to bind port 5038.
   Lesson: one ADB server, owned by the program, with clients pinned to it
   (`-H 127.0.0.1 -P <port>`), so nothing auto-spawns a second one.
5. **Validation broke the desktop**: `systemd-analyze --user verify` with the
   session's `XDG_RUNTIME_DIR` replaced the user manager's private socket.
   Lesson: tests never touch real systemd managers.

## What worked

- Manual USB toggle under the system manager (18:58:57 GMT): `mtp` then
  `rndis` re-enumerated, ADB came back authorized, first WAN check failed,
  check 10 s later passed. 30 s total, no manual help.
- Host-side NetworkManager profile down/up: ~6 s outage, ADB unaffected,
  WAN healthy after settling. 42 s total.
- Neither run ever needed its fallback restore, so **the fallback path has
  never been verified on hardware.**

## Techniques worth keeping

- Device selection: parse `adb devices -l`; accept only the configured serial
  with state `device` and a `usb:` field; take its `transport_id:N`; error if
  zero or more than one match. Re-select after every re-enumeration and run
  phone commands with `-t <transport_id>`.
- Phone commands as `adb ... shell -T -n <fixed args>`.
- USB state as a set: exactly one of `rndis`/`ncm` → on; neither → off;
  both → ambiguous; `adb` missing or format not `^[a-z0-9_]+(,[a-z0-9_]+)*$`
  → unknown.
- WAN health: global IPv4 on the interface (`ip -4 -o address show dev <if>
  scope global`) and `ping -4 -n -I <if> -c 1 -W <secs> 8.8.8.8`. Binding to
  the interface stops another uplink from hiding a dead phone WAN.
- Check WAN again right before any disruptive step; it may have recovered.
- Host-cycle test: confirm the active profile UUID and driver
  (`nmcli -g GENERAL.CON-UUID,GENERAL.DRIVER device show <if>`), and act only
  on that UUID with `ifname`.
- Interface discovery for install: walk `/sys/class/net/*/device` to the USB
  device and compare its `serial` with the phone serial.
- One run at a time: a lock file in the runtime directory.
- Per-run record: `status=running|passed|failed`, stage, exit code, times,
  written atomically (write temp, rename). A leftover `running` means the run
  was killed.
- Logs say what the program will do *before* it does it.

## Old defaults (for reference, not requirements)

| Setting | Old value | Note |
| --- | --- | --- |
| ADB server port | 5038 | separate from a desktop server on 5037 |
| ADB command timeout | 10 s | |
| Phone wait | 60 s | discovery + USB state read |
| Readiness wait | 120 s | `NetworkManager.service pc101-technitium.service` |
| Startup settle | 15 s | fixed sleep — replace with readiness events |
| Post-enable settle | 20 s | fixed sleep — replace with waiting for address + probe |
| Address wait | 45 s | |
| Probe timeout | 5 s | |
| Off/on pause | 1 s | replace with waiting for the observed off state |
| Final re-check | 10 s after first | |
| Whole run limit | 15 min (unit) | |
