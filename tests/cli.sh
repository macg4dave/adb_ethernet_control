#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
repo=$PWD
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
export ADB_BIN="$scratch/adb" ADB_TIMEOUT=2
export MOCK_LOG="$scratch/adb.log"
cat > "$ADB_BIN" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$MOCK_LOG"
if [[ "$*" == 'devices -l' ]]; then
    printf 'List of devices attached\n'
    case "${MOCK_SCENARIO:-one}" in
        one|partial)
            printf 'USB_A device usb:1-2 product:test model:Test transport_id:1\n'
            printf '192.0.2.1:5555 device product:test transport_id:2\n'
            ;;
        none) ;;
        unauthorized) printf 'USB_A unauthorized usb:1-2 transport_id:1\n' ;;
        offline) printf 'USB_A offline usb:1-2 transport_id:1\n' ;;
        multiple)
            printf 'USB_A device usb:1-2 transport_id:1\n'
            printf 'USB_B device usb:1-3 transport_id:2\n'
            ;;
        tcp) printf '192.0.2.1:5555 device product:test transport_id:2\n' ;;
        failed) exit 1 ;;
    esac
    exit 0
fi
[[ "$1" == -s && "$2" == USB_* && "$3" == shell ]] || exit 99
if [[ "${MOCK_SCENARIO:-one}" == partial && "$*" == *'dumpsys usb' ]]; then exit 1; fi
printf 'mock phone response\n'
MOCK
chmod +x "$ADB_BIN"
# Keep host probes deterministic and independent of the developer's network.
mkdir "$scratch/host"
for tool in ip nmcli; do
    printf '#!/usr/bin/env bash\nprintf "mock host response\\n"\n' > "$scratch/host/$tool"
    chmod +x "$scratch/host/$tool"
done
export PATH="$scratch/host:$PATH"

run_case() {
    local scenario=$1 expected=$2 actual=0
    shift 2
    : > "$MOCK_LOG"
    MOCK_SCENARIO=$scenario "$repo/bin/adb-ethernet-control" "$@" > "$scratch/output" 2>&1 || actual=$?
    if [[ "$actual" != "$expected" ]]; then
        cat "$scratch/output" >&2
        printf 'FAIL: scenario=%s expected=%s actual=%s\n' "$scenario" "$expected" "$actual" >&2
        exit 1
    fi
}
assert_contains() { grep -Fq -- "$1" "$scratch/output"; }
assert_no_shell() {
    if grep -q ' shell ' "$MOCK_LOG"; then
        printf 'FAIL: device probe ran after failed selection\n' >&2
        exit 1
    fi
}

run_case one 0 status
assert_contains 'ADB USB device: USB_A'
grep -Fq -- '-s USB_A shell -T -n dumpsys usb' "$MOCK_LOG"
if grep -q -- '-s 192.0.2.1' "$MOCK_LOG"; then exit 1; fi
run_case none 1
assert_contains 'no authorized USB phone'
assert_no_shell
run_case unauthorized 1 --serial USB_A
assert_contains 'unauthorized'
assert_no_shell
run_case offline 1 --serial USB_A
assert_contains 'offline'
assert_no_shell
run_case multiple 1
assert_contains 'multiple USB phones'
assert_no_shell
run_case multiple 0 --serial USB_B
assert_contains 'ADB USB device: USB_B'
run_case tcp 1 --serial 192.0.2.1:5555
assert_contains 'not a USB transport'
assert_no_shell
run_case one 1 --serial MISSING
assert_no_shell
run_case unauthorized 0 devices
assert_contains 'unauthorized'
assert_no_shell
run_case partial 2
assert_contains 'Report incomplete'
assert_contains 'Host network addresses'
run_case failed 1
assert_no_shell
run_case one 1 --serial
run_case one 1 --unknown
run_case one 1 status devices
run_case one 0 --help
ADB_TIMEOUT=bad run_case one 1
assert_contains 'positive integer'
printf 'CLI regression tests passed.\n'
