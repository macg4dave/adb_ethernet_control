#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin"
export PATH="$scratch/bin:$PATH" MOCK_LOG="$scratch/log" MOCK_STATE="$scratch/state"
export WAN_INTERFACE=testwan0 PHONE_SERIAL=TEST_PHONE ADB_SERVER_PORT=5038
export LOCK_FILE="$scratch/lock" STARTUP_DELAY=1 READINESS_TIMEOUT=1 PHONE_TIMEOUT=1
export POLL_INTERVAL=1 READY_UNITS=NetworkManager.service ADB_TIMEOUT=1
export ADB_BIN="$scratch/bin/adb" BOOT_ENV_FILE="$scratch/boot.env"
export USB_CYCLE_ENABLED=1 MOCK_CHECKS="$scratch/checks"
: > "$BOOT_ENV_FILE"
cat > "$scratch/bin/adb" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'adb %s\n' "$*" >> "$MOCK_LOG"
[[ "$1" == -P && "$2" == 5038 ]] || exit 99
shift 2
if [[ "$*" == 'devices -l' ]]; then
    case "$MOCK_SCENARIO" in
        absent) exit 0 ;;
        unauthorized) echo 'TEST_PHONE unauthorized usb:1-2 transport_id:8' ;;
        offline) echo 'TEST_PHONE offline usb:1-2 transport_id:8' ;;
        tcp) echo 'TEST_PHONE device transport_id:8' ;;
        ambiguous) printf 'TEST_PHONE device usb:1-2 transport_id:8\nTEST_PHONE device usb:1-3 transport_id:9\n' ;;
        *)
            echo 'OTHER_PHONE device usb:1-1 transport_id:7'
            if [[ $(< "$MOCK_STATE") == off ]]; then id=9; else id=8; fi
            echo "TEST_PHONE device usb:1-2 transport_id:$id" ;;
    esac
    exit 0
fi
[[ "$1" == -t && ( "$2" == 8 || "$2" == 9 ) && "$3" == shell && "$4" == -T && "$5" == -n ]] || exit 99
shift 5
case "$*" in
    'getprop sys.usb.state')
        if [[ "$MOCK_SCENARIO" == unknown ]]; then echo 'not-a-usb-state'; exit 0; fi
        if [[ $(< "$MOCK_STATE") == off ]]; then echo mtp,adb
        elif [[ "$MOCK_SCENARIO" == ncm ]]; then echo ncm,adb
        else echo rndis,adb; fi ;;
    'svc usb setFunctions mtp')
        [[ "$MOCK_SCENARIO" == toggle_no_effect ]] || echo off > "$MOCK_STATE" ;;
    'svc usb setFunctions rndis'|'svc usb setFunctions ncm')
        [[ "$MOCK_SCENARIO" != enable_fail ]] || exit 1
        echo enabled > "$MOCK_STATE" ;;
    *) exit 99 ;;
esac
MOCK
cat > "$scratch/bin/ip" <<'MOCK'
#!/usr/bin/env bash
printf '1: testwan0 inet 192.0.2.10/24 scope global testwan0\n'
MOCK
cat > "$scratch/bin/curl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'curl %s\n' "$*" >> "$MOCK_LOG"
[[ " $* " == *' --interface testwan0 '* && " $* " == *' --noproxy * '* ]] || exit 99
if [[ "$MOCK_SCENARIO" == healthy || "$MOCK_SCENARIO" == healthy_noadb ]]; then printf 200; exit 0; fi
if [[ "$MOCK_SCENARIO" == toggle* && $(< "$MOCK_STATE") == on ]]; then printf 200; exit 0; fi
if [[ "$MOCK_SCENARIO" == toggle_no_wan && $(< "$MOCK_STATE") == enabled ]]; then printf 000; exit 7; fi
if [[ $(< "$MOCK_STATE") == enabled && "$MOCK_SCENARIO" != persistent ]]; then
    count=$(< "$MOCK_CHECKS")
    echo "$((count + 1))" > "$MOCK_CHECKS"
    if [[ "$MOCK_SCENARIO" == regression && "$count" -ge 1 ]]; then printf 000; exit 7; fi
    if [[ "$MOCK_SCENARIO" != delayed || "$count" -ge 2 ]]; then printf 200; exit 0; fi
fi
printf 000
exit 7
MOCK
cat > "$scratch/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
printf 'systemctl %s\n' "$*" >> "$MOCK_LOG"
if [[ "$*" == *ControlGroup* ]]; then
    [[ "$MOCK_SCENARIO" != unsupervised ]] || { echo /unrelated-service; exit 0; }
    awk -F: '$1 == "0" || $2 == "name=systemd" { print $3; exit }' /proc/self/cgroup
fi
[[ "$MOCK_SCENARIO" != notready ]]
MOCK
cat > "$scratch/bin/systemd-run" <<'MOCK'
#!/usr/bin/env bash
printf 'arm fallback %s\n' "$*" >> "$MOCK_LOG"
[[ "$MOCK_SCENARIO" != arm_fail && "$MOCK_SCENARIO" != toggle_arm_fail ]]
MOCK
cat > "$scratch/bin/sleep" <<'MOCK'
#!/usr/bin/env bash
printf 'sleep %s\n' "$*" >> "$MOCK_LOG"
# Keep simulated startup and verification delays fast; wait_for still has a real deadline.
/usr/bin/sleep 0.02
MOCK
chmod +x "$scratch/bin/"*
run_case() {
    local scenario=$1 expected=$2 code=0
    shift 2
    : > "$MOCK_LOG"
    echo 0 > "$MOCK_CHECKS"
    echo on > "$MOCK_STATE"
    case "$scenario" in off|enable_fail) echo off > "$MOCK_STATE" ;; esac
    MOCK_SCENARIO=$scenario ./bin/adb-wan-boot "$@" > "$scratch/output" 2>&1 || code=$?
    if [[ "$code" != "$expected" ]]; then cat "$scratch/output" "$MOCK_LOG"; exit 1; fi
}
no_change() { if grep -q 'svc usb setFunctions' "$MOCK_LOG"; then exit 1; fi; }
run_case healthy 0
no_change
if grep -q 'adb\|sleep 10' "$MOCK_LOG"; then exit 1; fi
ADB_BIN=/nonexistent PHONE_SERIAL='' run_case healthy_noadb 0
no_change
run_case off 0
[[ $(grep -c 'svc usb setFunctions rndis' "$MOCK_LOG") == 1 ]]
if grep -q 'setFunctions mtp\|arm fallback' "$MOCK_LOG"; then exit 1; fi
[[ $(grep -c '^sleep 10$' "$MOCK_LOG") == 1 ]]
run_case stale 0
# Independent restore must be armed before off; exactly one second separates off/on.
awk '/arm fallback/ { armed=1 } /setFunctions mtp/ { if (!armed) exit 1; off=1 } /^sleep 1$/ { if (off) delay=1 } /setFunctions rndis/ { if (!off || !delay) exit 1; on=1 } END { if (!on) exit 1 }' "$MOCK_LOG"
[[ $(grep -c 'setFunctions mtp' "$MOCK_LOG") == 1 ]]
[[ $(grep -c 'setFunctions rndis' "$MOCK_LOG") == 1 ]]
grep -Fq 'adb -P 5038 -t 9 shell -T -n svc usb setFunctions rndis' "$MOCK_LOG"
grep -q 'systemctl stop .*timer' "$MOCK_LOG"
run_case delayed 0
[[ $(< "$MOCK_CHECKS") == 3 ]]
run_case regression 2
grep -Fq 'First WAN check: healthy' "$scratch/output"
[[ $(grep -c 'ERROR:' "$scratch/output") == 1 ]]
run_case persistent 2
[[ $(grep -c '^.*ERROR:' "$scratch/output") == 1 ]]
grep -Fq 'USB Ethernet on but WAN unavailable' "$scratch/output"
[[ $(grep -c 'setFunctions mtp' "$MOCK_LOG") == 1 ]]
run_case enable_fail 2
[[ $(grep -c 'setFunctions rndis' "$MOCK_LOG") == 1 ]]
grep -Fq 'USB Ethernet off but WAN unavailable' "$scratch/output"
USB_CYCLE_ENABLED=0 run_case stale 2
no_change
[[ $(grep -c 'ERROR:' "$scratch/output") == 1 ]]
USB_CYCLE_ENABLED=0 run_case off 0
[[ $(grep -c 'setFunctions rndis' "$MOCK_LOG") == 1 ]]
run_case ncm 0
grep -Fq -- '--restore ncm' "$MOCK_LOG"
grep -Fq 'setFunctions ncm' "$MOCK_LOG"
for scenario in absent unauthorized offline tcp ambiguous unknown notready arm_fail; do
    run_case "$scenario" 1
    no_change
done
# Exercise the independent timer's actual entrypoint, including enable-only semantics.
run_case off 0 --restore rndis
[[ $(grep -c 'setFunctions rndis' "$MOCK_LOG") == 1 ]]
run_case stale 0 --restore rndis
no_change
# Explicit manual test bypasses only the automatic cycle switch, requires supervision
# and a healthy baseline, and judges observed USB states independently from WAN.
export TEST_SYSTEMD_UNIT=adb-wan-usb-test ADB_SERVER_UNIT=''
USB_CYCLE_ENABLED=0 run_case toggle 0 --test-usb-toggle
[[ $(grep -c 'setFunctions mtp' "$MOCK_LOG") == 1 ]]
[[ $(grep -c 'setFunctions rndis' "$MOCK_LOG") == 1 ]]
grep -Fq 'off and on states verified; WAN healthy (informational)' "$scratch/output"
grep -Fq -- '--setenv=ADB_SERVER_PORT=5038' "$MOCK_LOG"
grep -Fq -- '--setenv=ADB_SERVER_UNIT=' "$MOCK_LOG"
USB_CYCLE_ENABLED=0 run_case toggle_no_wan 0 --test-usb-toggle
grep -Fq 'WAN unavailable (informational)' "$scratch/output"
run_case toggle_no_effect 1 --test-usb-toggle
grep -Fq 'off_observed=false' "$scratch/output"
run_case toggle_arm_fail 1 --test-usb-toggle
no_change
run_case stale 1 --test-usb-toggle
no_change
run_case unsupervised 1 --test-usb-toggle
no_change
TEST_SYSTEMD_UNIT='' run_case toggle 1 --test-usb-toggle
no_change
printf 'Phone USB boot recovery regression tests passed.\n'
