#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
repo=$PWD
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin"
export MOCK_LOG="$scratch/log" MOCK_STATE="$scratch/state"
export PHONE_SERIAL=TEST_PHONE WAN_INTERFACE=testwan0 ADB_SERVER_PORT=5037
export ADB_TIMEOUT=1 READINESS_TIMEOUT=1 PHONE_TIMEOUT=1 STARTUP_DELAY=1
export HEALTH_GRACE=1 RECOVERY_TIMEOUT=1 POLL_INTERVAL=1 OFF_DELAY=1 PROBE_TIMEOUT=1
export READY_UNITS='NetworkManager.service test-dhcp.service'
export LOCK_FILE="$scratch/lock" ADB_BIN="$scratch/bin/adb"
export PROBE_URLS='https://192.0.2.1/health https://192.0.2.2/health'
export PATH="$scratch/bin:$PATH"
cat > "$scratch/bin/adb" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'adb %s\n' "$*" >> "$MOCK_LOG"
[[ "$1" == -P && "$2" == 5037 ]] || exit 99
shift 2
if [[ "$*" == 'devices -l' ]]; then
    case "$MOCK_SCENARIO" in
        absent) printf 'List of devices attached\n' ;;
        unauthorized) printf 'TEST_PHONE unauthorized usb:1-2\n' ;;
        tcp) printf 'TEST_PHONE device transport_id:1\n' ;;
        wrong) printf 'OTHER_PHONE device usb:1-2\n' ;;
        *) printf 'TEST_PHONE device usb:1-2\n' ;;
    esac
    exit 0
fi
[[ "$1" == -s && "$2" == TEST_PHONE && "$3" == shell && "$4" == -T && "$5" == -n ]] || exit 99
shift 5
case "$*" in
    'getprop sys.usb.state')
        case "$MOCK_SCENARIO" in
            disabled) printf 'mtp,adb\n' ;;
            noadb) printf 'rndis\n' ;;
            combined) printf 'rndis,mtp,adb\n' ;;
            ncm) printf 'ncm,adb\n' ;;
            *) printf 'rndis,adb\n' ;;
        esac
        ;;
    'svc usb setFunctions')
        printf 'off\n' > "$MOCK_STATE"
        [[ "$MOCK_SCENARIO" != off_fail ]]
        ;;
    'svc usb setFunctions rndis'|'svc usb setFunctions ncm')
        [[ "$MOCK_SCENARIO" != restore_fail ]] || exit 1
        printf 'on\n' > "$MOCK_STATE"
        ;;
    *) exit 99 ;;
esac
MOCK
cat > "$scratch/bin/curl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'curl %s\n' "$*" >> "$MOCK_LOG"
[[ " $* " == *' --interface testwan0 '* && " $* " == *' --noproxy * '* ]] || exit 99
if [[ "$MOCK_SCENARIO" == healthy ]]; then printf 200; exit 0; fi
if [[ -f "$MOCK_STATE" && $(< "$MOCK_STATE") == on && "$MOCK_SCENARIO" != persistent ]]; then
    printf 200
    exit 0
fi
printf 000
exit 7
MOCK
cat > "$scratch/bin/ip" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == '-4 -o address show dev testwan0 scope global' ]] || exit 99
printf '1: testwan0 inet 192.0.2.10/24 scope global testwan0\n'
MOCK
cat > "$scratch/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'systemctl %s\n' "$*" >> "$MOCK_LOG"
[[ "$MOCK_SCENARIO" != notready ]]
MOCK
chmod +x "$scratch/bin/adb" "$scratch/bin/curl" "$scratch/bin/ip" "$scratch/bin/systemctl"

run_case() {
    local scenario=$1 expected=$2 actual=0
    shift 2
    : > "$MOCK_LOG"
    : > "$MOCK_STATE"
    MOCK_SCENARIO=$scenario "$repo/bin/adb-wan-recover" "$@" > "$scratch/output" 2>&1 || actual=$?
    if [[ "$actual" != "$expected" ]]; then
        cat "$scratch/output" >&2
        printf 'FAIL: recovery scenario=%s expected=%s actual=%s\n' "$scenario" "$expected" "$actual" >&2
        exit 1
    fi
}
no_toggle() {
    if grep -q 'svc usb setFunctions' "$MOCK_LOG"; then
        printf 'FAIL: unexpected phone mutation\n' >&2
        exit 1
    fi
}
one_cycle() {
    [[ $(grep -c 'svc usb setFunctions$' "$MOCK_LOG") == 1 ]]
    [[ $(grep -c 'svc usb setFunctions .*' "$MOCK_LOG") == 1 ]]
}

run_case healthy 0 --recover
no_toggle
grep -Fq 'is-active --quiet test-dhcp.service' "$MOCK_LOG"
run_case restore 0 --recover
one_cycle
grep -Fq 'WAN restored' "$scratch/output"
run_case ncm 0 --recover
one_cycle
grep -Fq 'setFunctions ncm' "$MOCK_LOG"
run_case off_fail 0 --recover
one_cycle
run_case persistent 2 --recover
one_cycle
run_case restore_fail 1 --recover
grep -Fq 'final best-effort restore' "$scratch/output"
run_case restore 2 --check
no_toggle
for scenario in notready absent unauthorized tcp wrong disabled noadb combined; do
    run_case "$scenario" 1 --recover
    no_toggle
done
run_case healthy 0 --help
no_toggle
PROBE_TIMEOUT=bad run_case healthy 1 --recover
no_toggle
run_case healthy 1 --invalid
printf 'WAN recovery regression tests passed.\n'
