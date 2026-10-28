# Boot WAN recovery

## Recovery policy

`adb-wan-recover --check` waits for configured services and checks connectivity
without changing USB functions. `--recover` can cycle USB networking once if
checks still fail and the configured phone reports both ADB and RNDIS/NCM.
All configuration comes from environment variables; scripts never evaluate
phone output or execute commands from configuration strings.

The system service starts after `network-online.target`. This target is an
ordering milestone, not proof of internet access, and can be reached after
wait-online fails. The script separately checks that every service in
`READY_UNITS` is active. A service with `Type=simple` may be active before its
application is fully ready; use the startup delay and the service's own
readiness mechanism where available. Do not make a readiness dependency wait
for this recovery service or for working WAN: that would create a dependency
cycle. The host's LAN DHCP/DNS service is separate from the phone's DHCP server,
which supplies the host's WAN address.

WAN health requires a global IPv4 address on `WAN_INTERFACE` and an HTTP 200
response from either configured HTTPS endpoint. Curl is bound to that
interface, ignores proxies and user curl configuration, and uses numeric IP
URLs by default to avoid a DNS dependency. Default probes contact Cloudflare
at `1.1.1.1` and `1.0.0.1`; replace them with endpoints known to work on your
carrier. A blocked endpoint, broken TLS/time setup, or a carrier outage can
cause a failed health check. Two endpoints reduce dependence on one address
but do not prove the fault is USB. Only one cycle is allowed per run.

Default waits: services 120 seconds, startup settling 15 seconds, initial WAN
45 seconds, ADB 60 seconds, USB-off delay 3 seconds, restored WAN 60 seconds.
Individual ADB calls are limited to 10 seconds and each HTTPS probe to 5
seconds. A wait can overrun its budget by its in-flight probe and polling
interval. The systemd job has a final 10-minute limit; increase it if you
increase the script's waits. A lock prevents overlapping runs sharing its
path. Direct CLI runs use the installed service's lock when its runtime
directory exists and is writable, otherwise the user's runtime directory.

The off command is `adb shell svc usb setFunctions` (charging/default
functions); the restore command is `adb shell svc usb setFunctions rndis`
or `ncm`, matching the original state. Combined modes containing extra USB
functions are rejected rather than silently dropping those functions.
Android normally adds ADB while USB
debugging is enabled, but vendor behavior must be verified. The script waits
for the same authorized USB serial after turning networking off and attempts
restoration even if the off command loses its response. A final bounded restore
is attempted on an error or termination during that interval. Loss of ADB,
power, or a hard kill can still require manual intervention on the phone.

Exit codes: `0` healthy or restored, `1` configuration/readiness/control failure,
`2` WAN unhealthy in check mode or still unhealthy after recovery. The service
does not restart recovery automatically; `RemainAfterExit=true` records a
successful run until the next boot or explicit restart.

## Prepare the host

Requirements: Bash, ADB, coreutils, iproute, curl, util-linux (`flock`), systemd,
and NetworkManager. Use the existing phone's authorized ADB key. Do not run the
ADB server as root or create a new identity if you expect existing authorization
to work. `User=dave` and `HOME=/home/dave` in the supplied system units match
this host; change both if deploying elsewhere.

The current `uaccess` rule grants USB access during an active local session.
Unattended boot needs a static group grant. The supplied udev rule grants
`adbwan` access only to HONOR USB devices exposing an ADB interface, including
product IDs that may change during a cycle. Both system services receive that
supplementary group; the login user does not need to be added to it.

Review these files before installing:

- `config/wan-recovery.env.example`
- `systemd/system/adb-wan-server.service`
- `systemd/system/adb-wan-recover.service`
- `udev/70-adb-wan.rules`

Run the following from the repository root to install files without enabling
or running recovery:

```bash
sudo groupadd --force adbwan
sudo install -Dm755 bin/adb-wan-recover /usr/local/bin/adb-wan-recover
sudo install -d -m755 /etc/adb-ethernet-control
sudo install -m640 -o root -g adbwan config/wan-recovery.env.example /etc/adb-ethernet-control/wan-recovery.env
sudo install -Dm644 udev/70-adb-wan.rules /etc/udev/rules.d/70-adb-wan.rules
sudo install -Dm644 systemd/system/adb-wan-server.service /etc/systemd/system/adb-wan-server.service
sudo install -Dm644 systemd/system/adb-wan-recover.service /etc/systemd/system/adb-wan-recover.service
sudoedit /etc/adb-ethernet-control/wan-recovery.env
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
```

Set `PHONE_SERIAL` from `adb devices -l` and confirm `WAN_INTERFACE`. Configure
`READY_UNITS` for the actual DHCP/DNS service; the example includes this host's
`pc101-technitium.service`. Set an absolute `ADB_BIN` path. The file uses plain
assignments with quoted space-separated values, no `export`. Keep device
serials in this host-local file, outside Git.

Apply the new udev rule to the phone by reconnecting it during a maintenance
window, or using a targeted udev trigger against its USB device sysfs path.
Verify the device node's group becomes `adbwan` and group read/write is allowed.
USB bus/device numbers can change.

The dedicated ADB server listens on localhost port 5038 with libusb and uses
the existing user's key. Only one ADB server can claim the phone interface.
Stop the old server with `adb kill-server` before starting the dedicated server:

```bash
adb kill-server
sudo systemctl start adb-wan-server.service
adb -P 5038 devices -l
```

For diagnostics once this server owns the phone, use `adb -P 5038 ...` or:

```bash
ADB_SERVER_SOCKET=tcp:localhost:5038 ./bin/adb-ethernet-control status
```

The recovery script passes `-P` explicitly. Do not enable a conflicting ADB
server at boot. Ensure the phone remains authorized after a reconnect and the
service user can access its existing key before testing recovery.

For explicit ordering after the host's LAN service, install a drop-in:

```ini
# /etc/systemd/system/adb-wan-recover.service.d/lan.conf
[Unit]
Wants=pc101-technitium.service
After=pc101-technitium.service
```

`READY_UNITS` still checks active status at runtime. The generic unit does not
hardcode a third-party DHCP service. Inspect dependencies before adding a
drop-in so startup cannot wait on itself.

## Validate before enabling

First run a read-only check using environment variables for this host. With
the original ADB server still running, use its port 5037; with the dedicated
server use 5038:

```bash
PHONE_SERIAL=YOUR_USB_SERIAL WAN_INTERFACE=enp48s0f4u2 \
  ADB_SERVER_PORT=5038 \
  READY_UNITS="NetworkManager.service pc101-technitium.service" \
  ./bin/adb-wan-recover --check
```

To read the installed environment as the service user without shell sourcing,
use a temporary system unit. Its name must differ from the installed service:

```bash
sudo systemd-run --unit=adb-wan-check --wait --pipe --collect \
  --property=User=dave --property=SupplementaryGroups=adbwan \
  --property=EnvironmentFile=/etc/adb-ethernet-control/wan-recovery.env \
  --property=RuntimeDirectory=adb-wan-check \
  --setenv=HOME=/home/dave --setenv=LOCK_FILE=/run/adb-wan-check/recovery.lock \
  /usr/local/bin/adb-wan-recover --check
```

A healthy check proves probing works, but does not exercise the USB toggle.
During a maintenance window, reproduce the reboot failure and start the
recovery service manually. Keep local access to the phone to restore USB
tethering if ADB is lost. Confirm the journal reports a cycle followed by
restored WAN, and that LAN DHCP/DNS still serves clients:

```bash
sudo systemctl start adb-wan-recover.service
journalctl -u adb-wan-server.service -u adb-wan-recover.service -b
```

After controlled recovery succeeds, enable boot recovery (without immediately
starting another cycle):

```bash
sudo systemctl enable adb-wan-recover.service
```

The server is started as a dependency; it does not need a separate enable.
Complete an attended reboot test before relying on unattended recovery. Verify
USB permissions before login, ADB authorization, the service order, WAN probes,
and that an already healthy boot performs zero USB changes. If a previous
successful run remains active, an explicit `systemctl restart` starts a new run;
there is no recurring recovery timer.

Disable future boot recovery with `sudo systemctl disable adb-wan-recover.service`.
Stopping a recovery in progress can interrupt its off/on sequence; let it
finish where possible. The older user reporting timer only prints diagnostics
and can be disabled independently.
