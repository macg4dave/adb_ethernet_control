#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin"
export MOCK_ROOT="$scratch/root" MOCK_LOG="$scratch/log"
export PATH="$scratch/bin:$PATH" PHONE_SERIAL=TEST_PHONE WAN_INTERFACE=testwan0
export ADB_BIN=/usr/local/bin/adb
cat > "$scratch/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "${MOCK_UNIT_STATE:-inactive}"
MOCK
cat > "$scratch/bin/restorecon" <<'MOCK'
#!/usr/bin/env bash
exit 99
MOCK
cat > "$scratch/bin/sudo" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$MOCK_LOG"
snapshot=/usr/local/bin/adb-wan-local-check
case "$1" in
    install)
        if [[ "$2" == -d ]]; then
            mkdir -p "$MOCK_ROOT$snapshot"
        else
            # Do not touch the host: stage the two source files under MOCK_ROOT.
            cp -- "${@:7:2}" "$MOCK_ROOT$snapshot/"
        fi
        ;;
    restorecon)
        [[ -f "$MOCK_ROOT$snapshot/adb-wan-test" && -f "$MOCK_ROOT$snapshot/adb-wan-recover" ]]
        ;;
    systemd-run)
        [[ "${@: -2:1}" == "$snapshot/adb-wan-test" && "${@: -1}" == --check ]]
        [[ " $* " == *' --setenv=USB_CYCLE_ENABLED=0 '* ]]
        [[ " $* " == *' --setenv=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin '* ]]
        grep -q '^restorecon ' "$MOCK_LOG"
        ;;
    *) exit 99 ;;
esac
MOCK
chmod +x "$scratch/bin/systemctl" "$scratch/bin/restorecon" "$scratch/bin/sudo"
./scripts/start-local-check > "$scratch/output"
cmp bin/adb-wan-test "$MOCK_ROOT/usr/local/bin/adb-wan-local-check/adb-wan-test"
cmp bin/adb-wan-recover "$MOCK_ROOT/usr/local/bin/adb-wan-local-check/adb-wan-recover"
: > "$MOCK_LOG"
code=0
MOCK_UNIT_STATE=activating ./scripts/start-local-check > "$scratch/output" 2>&1 || code=$?
[[ "$code" == 1 && ! -s "$MOCK_LOG" ]]
printf 'Launcher regression tests passed.\n'
