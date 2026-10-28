#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/caller-runtime/systemd" "$scratch/caller-config/systemd/user" "$scratch/bin"
# A regular file detects the real verifier's socket replacement without risking
# a live manager. The wrapper must preserve both its contents and inode.
printf 'caller private socket sentinel\n' > "$scratch/caller-runtime/systemd/private"
cp "$scratch/caller-runtime/systemd/private" "$scratch/expected"
inode=$(stat -c %i "$scratch/caller-runtime/systemd/private")
export XDG_RUNTIME_DIR="$scratch/caller-runtime" XDG_CONFIG_HOME="$scratch/caller-config"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$scratch/caller-bus"
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$scratch/caller-system-bus"
export NOTIFY_SOCKET="$scratch/caller-notify"
printf '[Service]\nExecStart=/usr/bin/true\n' > "$scratch/probe.service"
# The real verifier must also ignore broken overrides in the caller's config.
printf '[Service]\nExecStart=/nonexistent/command\n' > "$scratch/caller-config/systemd/user/probe.service"
./scripts/verify-user-units "$scratch/probe.service"
cmp "$scratch/expected" "$scratch/caller-runtime/systemd/private"
[[ $(stat -c %i "$scratch/caller-runtime/systemd/private") == "$inode" ]]
# Model the hazardous verifier behavior on success and failure, and assert that
# its runtime is private, disposable, and disconnected from both live buses.
export MOCK_LOG="$scratch/verifier-runtime"
cat > "$scratch/bin/systemd-analyze" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == '--user --generators=no verify '* ]]
[[ "$XDG_RUNTIME_DIR" != */caller-runtime && $(stat -c %a "$XDG_RUNTIME_DIR") == 700 ]]
[[ "$DBUS_SESSION_BUS_ADDRESS" == "unix:path=${XDG_RUNTIME_DIR%/runtime}/no-session-bus" ]]
[[ "$DBUS_SYSTEM_BUS_ADDRESS" == "unix:path=${XDG_RUNTIME_DIR%/runtime}/no-system-bus" ]]
[[ ! -v NOTIFY_SOCKET && -d "$SYSTEMD_ENVIRONMENT_GENERATOR_PATH" ]]
printf '%s\n' "$XDG_RUNTIME_DIR" > "$MOCK_LOG"
mkdir -p "$XDG_RUNTIME_DIR/systemd"
printf 'verifier replacement\n' > "$XDG_RUNTIME_DIR/systemd/private"
exit "${MOCK_EXIT:-0}"
MOCK
chmod +x "$scratch/bin/systemd-analyze"
for expected in 0 1; do
    code=0
    PATH="$scratch/bin:$PATH" MOCK_EXIT=$expected ./scripts/verify-user-units "$scratch/probe.service" || code=$?
    [[ "$code" == "$expected" && ! -e $(< "$MOCK_LOG") ]]
    cmp "$scratch/expected" "$scratch/caller-runtime/systemd/private"
done
printf 'User unit verification isolation regression tests passed.\n'
