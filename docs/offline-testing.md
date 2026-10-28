# Tests that continue while WAN is down

The AI connection shares this host's phone WAN. A live test must not depend
on the AI, the IDE, a remote terminal, or a second tool call to finish a stage
or restore networking. Losing chat connectivity is expected during a cycle.

## Local supervision and records

`adb-wan-test` owns the complete local sequence: pre-test snapshot, one bounded
recovery/check command, post-test snapshot, and a final result. Failed ADB,
missing interfaces, and failed recovery do not skip the post-test snapshot.
There are no chat callbacks or online dependencies in this orchestration.
Only the WAN probes intentionally need internet access.

Run it through the **system** manager using `adb-wan-test@check.service`.
The service runs as `dave`, independent of an interactive login or IDE
connection. It remains local when WAN disappears. Do not rely on `nohup`
launched by an AI tool as proof that the operating system owns the test.
The test units are started manually and have no boot enablement section.

Each run writes a private directory under `/var/log/adb-wan-test/` containing:

- `output.log`: stdout/stderr from every stage, plus timestamps and snapshots.
- `result`: atomically updated `status`, `stage`, `mode`, `exit_code`, and times.

The result starts as `running`, then ends as `passed` or `failed`. A result
left `running` means the run is incomplete, for example after a hard kill or
power loss; it must never be interpreted as a pass. Ordinary termination
signals record failure through an exit trap. Logs are retained across network
loss and reboots, and can contain device/network identifiers; keep them out
of Git.

The recovery command has a default 600-second hard timeout. Post-test
diagnostics still run on timeout, with exit code `124` recorded. Individual
diagnostics are limited to 10 seconds. The service allows 15 minutes for the
entire sequence and cleanup; adjust both limits together if increasing them.

## Install and start a read-only test

To test the supervisor now, without installing any boot services, run this
from a **local terminal** in the repository:

```bash
WAN_INTERFACE=enp48s0f4u2 \
  READY_UNITS="NetworkManager.service pc101-technitium.service" \
  ./scripts/start-local-check
```

This asks sudo to authenticate locally, installs root-owned copies of the
runner and recovery script under `/usr/local/bin/adb-wan-local-check/`,
applies the normal SELinux labels with `restorecon` when available, then
submits a read-only job to the system manager and returns. Launching directly
from the home checkout failed with `203/EXEC` on this RHEL host; the launcher
now executes the installed snapshot instead. The installed path has the
normal RHEL `bin_t` executable label; SELinux remains enabled.
The managed PATH includes `/usr/sbin` and `/sbin` for RHEL's `ip` executable;
the launcher checks required recovery commands against this PATH before
installing or submitting the job.
It selects the unique USB phone unless
`PHONE_SERIAL` is provided, uses the current ADB server (port 5037 by default),
and forces `USB_CYCLE_ENABLED=0`. No services are enabled at boot. The job
continues if chat or the terminal disconnects. Use
`journalctl -u adb-wan-local-check` and the saved result files to inspect it.
The transient unit is collected after completion, while disk records remain.
The test uses its installed snapshot, so checkout edits cannot change a run
in progress. The launcher refuses to replace that snapshot while a check is
active. It does not install or enable any boot service.

For a permanent test unit:

First prepare the host-local configuration, USB group/rule, and dedicated ADB
server from [boot recovery](boot-recovery.md). Those instructions require sudo
in a local terminal. Set `USB_CYCLE_ENABLED=0`; it stays disabled after both
failed HONOR tests. Then install the runner and service from the repository:

```bash
sudo install -Dm755 bin/adb-wan-test /usr/local/bin/adb-wan-test
sudo install -Dm644 systemd/system/adb-wan-test@.service /etc/systemd/system/adb-wan-test@.service
sudo systemctl daemon-reload
sudo systemctl start --no-block adb-wan-test@check.service
```

`--no-block` returns after systemd queues the job. The host performs the
remaining steps without this chat. Inspect it from a local terminal or after
the connection returns:

```bash
systemctl status adb-wan-test@check.service --no-pager
journalctl -u adb-wan-test@check.service
sudo ls -lt /var/log/adb-wan-test/
sudo cat /var/log/adb-wan-test/run-XXXXXXXX/result
sudo cat /var/log/adb-wan-test/run-XXXXXXXX/output.log
```

Replace `run-XXXXXXXX` with the directory shown in the journal. A completed
oneshot unit can become inactive; use its exit status and saved `result` to
judge success. A later explicit start runs a fresh test with a new log
directory. Do not increase `TEST_TIMEOUT` past the unit limit.

For development, direct execution also records every stage:

```bash
PHONE_SERIAL=YOUR_USB_SERIAL WAN_INTERFACE=enp48s0f4u2 \
  ./bin/adb-wan-test --check
```

Direct execution defaults to `~/.local/state/adb-ethernet-control/tests/`.
This is useful for checking the runner, but does not establish independent
systemd supervision. `TEST_LOG_ROOT` and `RECOVERY_BIN` can override paths.

## WAN restoration is a separate requirement

Keeping a test running offline does not make an unreliable phone toggle safe.
The last two live cycles required manual restoration. The detached phone
worker did not reach its restore result, and the exact reason is unresolved.
The `test-cycle` instance is therefore blocked by `USB_CYCLE_ENABLED=0`.
Do not enable it merely to test the supervisor.

Before another disruptive test, establish a verified restoration mechanism
that survives the USB change. The entire fallback must be scheduled locally
before networking is touched. A host supervisor cannot send ADB commands
through an interface that no longer exists. Keep a local person able to
restore the phone until that fallback has been verified. There are no further
automatic cycle retries and no reboot action in this test runner.
