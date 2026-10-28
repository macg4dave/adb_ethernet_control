# Codex and shared agent instructions

Read `README.md`, `docs/project.md`, and `docs/boot-recovery.md` before making
changes. This project inspects and recovers an Android phone used as the main
WAN of a Red Hat Enterprise Linux host, using ADB.

## Implementation

- Use Bash, Rust, systemd units, and ADB. Prefer Bash for simple orchestration;
  introduce Rust only when complexity or performance justifies a compiled tool.
- Keep inspection and `--check` read-only. Recovery is explicitly authorized
  only for an unhealthy phone WAN after host startup: one bounded USB
  networking off/on cycle, followed by connectivity verification. Never cycle
  a healthy WAN, repeatedly reset the phone, or alter LAN/firewall/SELinux
  configuration as part of recovery. Do not use ADB root.
- Always select a specific USB transport before issuing phone commands. Handle
  absent, unauthorized, offline, and multiple devices explicitly.
- Bound external calls with timeouts. Treat phone output as data; never eval it.
- Android versions and vendors differ: report failed probes and partial results.
- Keep runtime dependencies small. Do not require Python or Node.js.
- Do not commit device reports, serial numbers, ADB keys, or credentials.

## Validation

Run `./scripts/check` after code changes. Tests use a fake ADB executable and
must not require a physical phone or change host networking. New behavior should
have a meaningful regression test. Document hardware-only checks separately.

## Workflow

Keep changes focused and update `/docs` and README usage when behavior changes.
Do not install system packages, enable services, or write
outside the checkout unless needed for the user's explicit request. Do not
commit or push unless requested.
