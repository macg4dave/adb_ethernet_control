# Project purpose

The HONOR Android phone is the RHEL host's main WAN connection. The host also
serves a LAN, including DHCP/DNS. Following a host restart, the phone can still
show USB Ethernet/tethering enabled while the host cannot use the WAN. Manually
turning USB tethering off and back on is the recovery behavior to automate.

Implementation uses Bash, ADB, and systemd; Rust remains available if future
complexity warrants it. No Rust runtime or build is required today.

## Intended boot behavior

1. Start after host network startup has been attempted.
2. Wait for NetworkManager and configured LAN services, such as DHCP/DNS, to
   be active, then allow a settling interval.
3. Give the phone WAN time to acquire an address and pass connectivity checks.
4. If WAN is healthy, finish without changing anything.
5. If WAN remains unhealthy, find the configured, authorized USB phone and
   read its actual USB functions.
6. If USB Ethernet is off, enable the configured RNDIS/NCM function once. If
   it is on but WAN is down, cycle off/on once with a one-second pause, only
   after independent restoration has been verified and cycling enabled.
7. After enabling, allow 20 seconds for tethering and up to 45 seconds for
   host IPv4 addressing. Ping `8.8.8.8` through the phone interface, wait ten
   seconds, and ping again. On final failure, log one
   error with the USB state and stop without further cycles.

The boot service does not repeatedly reset a phone during a carrier outage.
It does not change the LAN bridges, firewall, DHCP configuration, or routing
profiles. NetworkManager remains responsible for addressing and route setup.

## Findings on this host

During inspection on 28 October 2026:

- HONOR ALI-NX1 runs Android 15 and exposes `rndis,adb` when tethering; Android init also defines vendor sets
  such as `hisuite,mtp,mass_storage,adb` and `mtp,adb,hdb`.
- NetworkManager uses `honor-usb-wan` on `enp48s0f4u2`, with autoconnect and
  IPv4 DHCP enabled.
- `pc101-technitium.service` is the running LAN DNS/DHCP service.
- `NetworkManager-wait-online.service` failed during the current boot. This
  establishes a startup readiness problem, but does not prove the cause of
  the reported WAN failure.
- Restarting ADB with libusb restored USB discovery. Phone authorization was
  then accepted, and the diagnostic CLI completed successfully.
- The phone advertises `svc usb setFunctions` and reports `rndis` from
  `svc usb getFunctions`. Its `cmd usb` and `cmd tethering` services do not
  implement shell commands.

The first live cycle failed: switching off USB networking also removed ADB,
so the host could not issue its restore command. The user restored tethering
manually. The user also observed a 10–15 second delay between enabling USB
Ethernet and WAN availability. Recovery now schedules the entire off/wait/on
sequence on the phone and allows a configurable 20-second settling interval
before WAN retries. A non-disruptive detached phone-worker probe succeeded.
However, the second live cycle also required manual restoration. Its worker
log ended during the off command, without an off-state or restore result.
This does not establish whether the command blocked or the worker was killed,
but shows that detached shell execution alone did not solve restoration.

USB cycling is now disabled by default (`USB_CYCLE_ENABLED=0`). Normal checks
and healthy-WAN no-op behavior remain available. The USB-based experimental
recovery must not be enabled until its restoration mechanism is verified.
The earlier boot service used conditional host-side NetworkManager
reconnection. Following the reboot failure below, the boot code now selects
the phone's USB state and supports enable-only recovery. A guarded off/on
branch arms an independent systemd restore timer, pauses one second between
off/on, and checks WAN twice ten seconds apart. Its hardware behavior remains
unverified, so cycling remains disabled by default. See the boot guide.
An explicit manual `--test-usb-toggle` flag now exercises one supervised USB
off/on cycle independently of that automatic switch. It requires a healthy
WAN baseline and observed USB off/on states; final WAN results are recorded
separately and do not determine that test's pass/fail.

A separate manually launched host-side test now exercises NetworkManager
profile down/up while preserving the phone's USB mode. Its independent systemd
restore timer is armed before the outage. It has mock coverage for coordinator
failure, interruption, and failed activation. The live systemd-managed test
passed on 28 October 2026: the profile was deactivated, the host-side outage
was observed while ADB remained available, and the profile reactivated about
six seconds later. After the 20-second settling interval, WAN probes passed.
The complete run took 42 seconds with exit code 0. The independent fallback
timer was armed before the outage and canceled after successful activation;
the fallback itself did not need to fire. No phone USB functions changed.

The attended reboot test on 28 October 2026 failed to restore WAN automatically.
The phone was present on USB at startup, but no RNDIS network interface was
exposed. The user manually enabled tethering while the boot service was still
waiting for initial connectivity; the interface then appeared and WAN probes
passed. The service performed no reconnection or USB changes. Its `passed`
record therefore describes connectivity after manual intervention, not a
successful automatic recovery. The evidence does not establish why tethering
was disabled across the reboot. Host-side NetworkManager reconnection cannot
restore a network interface that the phone does not expose.

The attended manual USB toggle test on 28 October 2026 at 18:58:57 GMT
passed in 30 seconds, with no manual intervention confirmed by the user.
The system-managed test observed USB Ethernet off and on, retained authorized
ADB after re-enumeration, and recorded healthy WAN at the final check ten
seconds after the first failed check. Its independent restore timer was armed
but canceled before firing. This verifies one MTP/RNDIS cycle on the phone;
the fallback's actual restoration path remains unverified, so automatic cycling
stays disabled. See [the hardware record](offline-testing.md#explicit-usb-toggle-test).

Automatic WAN recovery and unattended cold-boot behavior have **not** been verified.

The later reboot at 19:07 GMT on 28 October also needed manual tethering
enablement, confirmed by the user. Its boot record began with no WAN interface
and ended with `WAN recovered before USB changes`; no enable command was sent.
A root-owned ADB daemon had started in the recovery service while the dedicated
user-owned server repeatedly failed to bind port 5038. Recovery now uses an
explicit loopback ADB host to prevent client auto-spawning, retries discovery
while the dedicated server starts, and logs the observed USB state. Mock tests
cover delayed server availability and startup failure. The corrected startup
path still requires deployment and attended reboot validation.

Changing USB functions is not the same API as changing the Settings tethering
switch. This phone must be tested to establish that cycling RNDIS causes its
tethering service and DHCP server to recover, and that ADB survives the cycle.

## Components and acceptance criteria

`bin/adb-ethernet-control` provides read-only reports. `bin/adb-wan-recover`
implements check/recovery modes. The boot units live in `systemd/system/`;
the older units under `systemd/user/` are optional diagnostic reporting only.

Recovery is ready for unattended use when a controlled test confirms all of:

- A healthy WAN causes zero USB changes.
- After a host reboot, recovery runs without an interactive login.
- A stale USB tethering state can be restored with one off/on cycle.
- The phone retains authorized ADB access across USB re-enumeration.
- NetworkManager reconnects the existing WAN profile and obtains an address,
  default route, and working WAN reachability.
- Missing/unauthorized phones and carrier outages produce bounded failures
  without repeated toggles or changes to LAN services.

See [boot recovery](boot-recovery.md) for deployment and validation.
