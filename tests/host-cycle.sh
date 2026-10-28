#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin"
export PATH="$scratch/bin:$PATH" MOCK_LOG="$scratch/log" MOCK_STATE="$scratch/state"
export MOCK_CHECKS="$scratch/checks" WAN_INTERFACE=testwan0 PHONE_SERIAL=TEST_PHONE
export WAN_CONNECTION_UUID=11111111-2222-3333-4444-555555555555
export ADB_BIN="$scratch/bin/adb" RECOVERY_BIN="$scratch/bin/recover"
cat > "$scratch/bin/nmcli" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'nmcli %s\n' "$*" >> "$MOCK_LOG"
case "$*" in
    *GENERAL.CON-UUID*)
        if [[ $(< "$MOCK_STATE") == up ]]; then printf '%s\n' "$WAN_CONNECTION_UUID"; else printf '%s\n' --; fi ;;
    *GENERAL.DRIVER*)
        if [[ "$MOCK_SCENARIO" == bad_driver ]]; then echo bridge; else echo rndis_host; fi ;;
    *'connection down uuid '*)
        [[ "$MOCK_SCENARIO" != down_fail ]] || exit 1
        if [[ "$MOCK_SCENARIO" != no_outage ]]; then echo down > "$MOCK_STATE"; fi ;;
    *'connection up uuid '*)
        [[ "$MOCK_SCENARIO" != up_fail ]] || exit 1
        echo up > "$MOCK_STATE" ;;
    *) exit 99 ;;
esac
MOCK
cat > "$scratch/bin/adb" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == *'getprop sys.usb.state' ]] || exit 99
printf 'adb read USB state\n' >> "$MOCK_LOG"
if [[ "$MOCK_SCENARIO" == adb_fail && $(< "$MOCK_STATE") == down ]]; then exit 1; fi
echo rndis,adb
MOCK
cat > "$scratch/bin/recover" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == --check && "$USB_CYCLE_ENABLED" == 0 ]] || exit 99
count=$(< "$MOCK_CHECKS")
count=$((count + 1))
echo "$count" > "$MOCK_CHECKS"
printf 'check %s\n' "$count" >> "$MOCK_LOG"
if [[ "$MOCK_SCENARIO" == baseline_fail && "$count" == 1 ]]; then exit 2; fi
if [[ "$MOCK_SCENARIO" == after_fail && "$count" == 2 ]]; then exit 2; fi
MOCK
cat > "$scratch/bin/systemd-run" <<'MOCK'
#!/usr/bin/env bash
printf 'arm fallback %s\n' "$*" >> "$MOCK_LOG"
[[ "$MOCK_SCENARIO" != arm_fail ]]
MOCK
cat > "$scratch/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
printf 'cancel fallback %s\n' "$*" >> "$MOCK_LOG"
MOCK
cat > "$scratch/bin/sleep" <<'MOCK'
#!/usr/bin/env bash
if [[ "$MOCK_SCENARIO" == interrupted ]]; then kill -TERM "$PPID"; fi
MOCK
chmod +x "$scratch/bin/"*
run_case() {
    local scenario=$1 expected=$2 code=0
    : > "$MOCK_LOG"
    echo up > "$MOCK_STATE"
    echo 0 > "$MOCK_CHECKS"
    MOCK_SCENARIO=$scenario ./bin/adb-wan-host-cycle > "$scratch/output" 2>&1 || code=$?
    if [[ "$code" != "$expected" ]]; then cat "$scratch/output"; exit 1; fi
}
run_case healthy 0
grep -Fq 'PASS: host outage observed' "$scratch/output"
[[ $(< "$MOCK_CHECKS") == 2 && $(< "$MOCK_STATE") == up ]]
# The independent restore must be scheduled before the down command.
awk '/arm fallback/ { armed=1 } /connection down/ { if (!armed) exit 1; down=1 } END { if (!down) exit 1 }' "$MOCK_LOG"
for scenario in down_fail no_outage adb_fail; do
    run_case "$scenario" 1
    [[ $(< "$MOCK_STATE") == up ]]
    grep -q 'cancel fallback' "$MOCK_LOG"
done
run_case up_fail 1
if grep -q 'cancel fallback' "$MOCK_LOG"; then exit 1; fi
run_case interrupted 143
[[ $(< "$MOCK_STATE") == up ]]
run_case after_fail 2
[[ $(< "$MOCK_STATE") == up ]]
for scenario in baseline_fail bad_driver arm_fail; do
    expected=1
    if [[ "$scenario" == baseline_fail ]]; then expected=2; fi
    run_case "$scenario" "$expected"
    if grep -q 'connection down' "$MOCK_LOG"; then exit 1; fi
done
printf 'Host-side cycle regression tests passed.\n'
