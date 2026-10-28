---
paths:
  - "src/**/*.rs"
  - "tests/**/*.rs"
  - "Cargo.toml"
  - "Cargo.lock"
---

# Rust rules

This is a small, safety-critical system tool that runs as root at boot with
the network possibly down. Prefer boring, explicit, testable code over
clever code.

## Toolchain and build

- Stable Rust, edition 2024, MSRV in `Cargo.toml` (`rust-version`). Do not use
  nightly features.
- Commit `Cargo.lock` (this is a binary).
- Must pass: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`. Do not silence a lint with `#[allow]` unless the reason is
  written next to it: `#[allow(clippy::x, reason = "...")]`.
- `unsafe` is forbidden (`unsafe_code = "forbid"` in `Cargo.toml`).
- The build must work offline from the vendored lockfile once dependencies are
  fetched; never add `build.rs` scripts that download anything.

## Dependencies

- Keep them few. Each new crate needs a one-line justification in the commit
  or PR. Prefer std. Never add a crate for something std does in ~20 lines.
- Acceptable when needed: `serde` (derive) + `toml` for the config file,
  `clap` (derive) for the CLI, `thiserror` for error types. Ask before adding
  anything else.
- **No async runtime** (no tokio/async-std). The tool is sequential; use
  `std::process`, `std::thread` and `std::time`.
- No crates that link C libraries (no libusb/libsystemd bindings). Talk to the
  system through its command-line tools, called via `crate::process`.

## Architecture

Single binary `adb-wan` with subcommands. Layering, top to bottom:

1. `main.rs`: parse CLI, call one command, map the error to an exit code.
   No logic.
2. `commands/`: one file per subcommand; orchestrates a flow.
3. Domain modules (`adb/`, `net/`, `units/`, `recovery/`, `record.rs`,
   `config.rs`): capabilities and pure logic.
4. `process.rs`: the **only** place that spawns external programs.

Rules:

- Lower layers never import from higher layers. `commands/*` never import each
  other.
- Decisions are **pure functions** on plain data
  (`recovery::decision`): given USB state, WAN result and config, return an
  action. No I/O in them. This is where most unit tests go.
- I/O sits behind small traits (e.g. `trait Adb`, `trait Network`,
  `trait Clock`) so commands can be tested with fakes. Pass dependencies in;
  no global state, no `lazy_static`/`OnceLock` singletons for services.
- Model states as enums, not strings or bools: `UsbEthernet::{On, Off,
  Unknown(String)}`, `DeviceState::{Device, Unauthorized, Offline, ...}`.
  Make illegal states unrepresentable; parse at the boundary, then trust the
  type.
- Newtypes for identifiers: `Serial`, `TransportId`, `InterfaceName`.
  Validate on construction. Never log or commit a full real serial in tests;
  use obvious fakes like `FAKE0SERIAL`.

## External processes (`process.rs`)

- Every call has an explicit timeout; on expiry kill the child and return an
  error. No call without a timeout, ever.
- Use `Command::new(program).args([...])`. Never `sh -c`, never build a
  command string, never interpolate phone output into arguments.
- Clear or set the environment explicitly where it matters (e.g. set
  `ADB_SERVER_SOCKET=tcp:127.0.0.1:<port>` for every `adb` call so the client
  never auto-spawns a server).
- The ADB server is a child process owned by `adb::server`: started only when
  recovery needs the phone, waited for with a deadline, and killed on every
  exit path (implement `Drop` on its guard).
- Capture stdout/stderr, cap their size, and treat them as untrusted. Parse
  with strict, tested parsers; reject unexpected formats with an error that
  includes the (truncated) raw text.
- Program paths come from the config file. Code that uses `process.rs` sits
  behind a trait so tests replace it in-process; tests never run programs.

## Errors and exit codes

- Library code returns `Result<T, Error>` with a `thiserror` enum per area
  (or one crate-level enum). Errors carry context: what was attempted, on
  which device/interface, and the raw output if relevant.
- No `unwrap()`, `expect()`, `panic!`, `todo!`, or indexing that can panic in
  non-test code. `expect` is allowed only for true invariants, with a message
  stating the invariant.
- Map errors to the documented exit codes (0/1/2) in exactly one place
  (`error.rs` + `main.rs`).
- Never swallow an error silently. If something is intentionally ignored,
  log it and comment why.

## Logging

- Write logs to stderr, one line per event, so journald captures them under
  the unit. Prefix with sd-daemon priorities (`<3>` error, `<4>` warning,
  `<6>` info, `<7>` debug) instead of pulling in a logging framework.
- Log every decision and every state-changing action *before* doing it, with
  the inputs that led to it. A reader of `journalctl -b -t adb-wan` must be
  able to reconstruct what happened; boot behavior is debugged from logs
  alone. Follow "Boot run: logging is the test" in `AGENTS.md` (first line,
  stage start/end with elapsed ms, raw observations, wait outcomes, one
  final `result=...` summary line).
- Use `key=value` fields after a short message
  (`<6>usb state read raw="mtp,adb" parsed=off elapsed_ms=812`) so lines are
  greppable. Quote values that may contain spaces.
- Stage logging goes through one small helper (e.g. a `Stage` guard that
  logs start, and on drop logs end with elapsed time and outcome), so no
  stage can forget its end line.
- Per-run records (boot and test) are written by `record.rs` to
  `/var/log/adb-wan/<kind>-<UTC time>/`: a `result` file (written atomically,
  `status=running` first) and an `events` file with the same lines as the
  journal. A leftover `status=running` means the run was killed.
- Never log ADB keys or secrets. Serials may be logged on the host, never put
  in committed fixtures.

## Time and waiting (event-driven)

- Never sleep a fixed time to "let things settle". Wait for an observable
  condition: unit active, transport `device`, valid `sys.usb.state`,
  interface present, global IPv4 present, probe passing.
- Use one helper, `wait_until(condition, deadline)`, for every wait. It checks
  the condition, then re-checks at `timing.poll_interval_secs` until true
  or the deadline passes. It returns which happened; it never loops forever.
  Prefer a blocking primitive when one exists (e.g. `adb wait-for-usb-device`
  under a timeout) over polling.
- Deadlines come from `Config` with documented defaults, never magic numbers.
  The only intentional pause is the post-recovery confirm interval
  (`confirm_interval_secs`), which is a stability check, not a settle delay.
- Inject a `Clock` in flows so tests run instantly.

## Configuration

- Every setting lives in one TOML file,
  `/etc/adb-ethernet-control/config.toml` (path overridable only by a
  `--config` flag). Never read settings from environment variables.
- `config.rs` deserializes it once at startup into typed structs with
  `#[serde(deny_unknown_fields)]`, then validates (ranges, non-empty serial,
  interface name format). Errors name the key and the bad value.
- Group keys into tables by area (`[phone]`, `[wan]`, `[adb]`, `[timing]`,
  `[recovery]`, `[readiness]`, `[logging]`). Durations are integer seconds
  with a `_secs` suffix and convert to `Duration` at load.
- Every optional key has a documented default in code and in the committed
  example `config/config.example.toml`; keep the two in sync (a unit test
  parses the example).
- Safe defaults: `recovery.usb_cycle_enabled = false`.
- Other modules receive `&Config` (or the sub-struct they need); none read
  files or `std::env` for settings.

## Testing

- Unit tests in the same file (`#[cfg(test)] mod tests`) for parsers and
  decisions. Cover every enum variant and every documented edge case
  (empty `sys.usb.state`, vendor sets like `hisuite,mtp,mass_storage,adb`,
  multiple devices, unauthorized device, timeout).
- All tests are Rust (`cargo test`). Flow tests in `tests/*.rs` drive the
  library's commands with in-process fakes of the I/O traits (scripted
  device lists, USB states, probe results, a fake clock).
- Tests never spawn external programs: no fake executables, no `adb`,
  `nmcli`, `systemctl`, `systemd-analyze`, no shell. They need no root, phone,
  network or systemd manager.
- The committed unit file and example config are checked by Rust tests that
  read and parse them as text/TOML (required directives present, forbidden
  ones absent).
- Every bug fix comes with a test that failed before the fix.
- Tests are deterministic: no real sleeps over a few milliseconds, no real
  network, no dependence on test order.

## Style

- `rustfmt` defaults. Names: modules `snake_case`, types `UpperCamelCase`.
- Small functions, early returns, no deep nesting. Prefer `match` on enums
  over `if` chains on strings.
- Public items get a one-line `///` doc saying *what* and any safety-relevant
  *why*. Every module starts with a `//!` line saying what it owns.
- Comments explain why (often: which hardware test motivated a rule), not
  what the code does.
- No dead code, no commented-out code, no speculative abstractions.
