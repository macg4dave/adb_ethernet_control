#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/repo/scripts" "$scratch/bin"
cp install.sh "$scratch/repo/install.sh"
export MOCK_LOG="$scratch/log" PATH="$scratch/bin:$PATH"
export PHONE_SERIAL=TEST_PHONE WAN_INTERFACE=testwan0 ADB_BIN="$scratch/bin/adb"
export READY_UNITS=NetworkManager.service
export INSTALL_CONFIG_FILE="$scratch/boot.env" INSTALL_SYS_CLASS_NET="$scratch/net"
mkdir "$INSTALL_SYS_CLASS_NET"
cat > "$scratch/bin/id" <<'MOCK'
#!/usr/bin/env bash
echo "${MOCK_UID:-1000}"
MOCK
cat > "$scratch/repo/scripts/install-boot-service" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$PHONE_SERIAL" "$WAN_INTERFACE" "$ADB_BIN" "$READY_UNITS" > "$MOCK_LOG"
exit "${MOCK_INSTALL_STATUS:-0}"
MOCK
for tool in adb sudo systemctl systemd-analyze timeout ping ip flock udevadm; do
    printf '#!/usr/bin/env bash\nexit 0\n' > "$scratch/bin/$tool"
done
cat > "$scratch/bin/timeout" <<'MOCK'
#!/usr/bin/env bash
shift 2
exec "$@"
MOCK
cat > "$scratch/bin/adb" <<'MOCK'
#!/usr/bin/env bash
if [[ "$*" == '-H 127.0.0.1 -P 5038 -t 8 shell -T -n getprop sys.usb.state' ]]; then
    [[ -n ${MOCK_USB_STATE:-} ]] || exit 1
    printf '%s\r\n' "$MOCK_USB_STATE"
    exit 0
fi
[[ "$*" == '-H 127.0.0.1 -P 5038 devices -l' || "$*" == '-H 127.0.0.1 -P 5037 devices -l' ]] || exit 99
case "${MOCK_DISCOVERY:-none}" in
    one) echo 'AUTO_PHONE device usb:1-2 transport_id:8' ;;
    pinned) echo 'TEST_PHONE device usb:1-2 transport_id:8' ;;
    multiple) printf 'AUTO_PHONE device usb:1-2 transport_id:8\nOTHER_PHONE device usb:1-3 transport_id:9\n' ;;
    unauthorized) echo 'AUTO_PHONE unauthorized usb:1-2 transport_id:8' ;;
    offline) echo 'AUTO_PHONE offline usb:1-2 transport_id:8' ;;
    tcp) echo '192.0.2.1:5555 device transport_id:8' ;;
    failed) exit 1 ;;
esac
MOCK
cat > "$scratch/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
if [[ ${MOCK_SERVICE:-missing} == loaded ]]; then echo loaded; else echo not-found; fi
MOCK
chmod +x "$scratch/bin/"* "$scratch/repo/scripts/install-boot-service"
run_case() {
    local expected=$1 code=0
    shift
    rm -f "$MOCK_LOG"
    "$scratch/repo/install.sh" "$@" </dev/null > "$scratch/output" 2>&1 || code=$?
    if [[ "$code" != "$expected" ]]; then cat "$scratch/output"; exit 1; fi
}
run_case 0 --help
[[ ! -e "$MOCK_LOG" ]]
run_case 0 --dry-run
grep -Fq 'Preview complete.' "$scratch/output"
[[ ! -e "$MOCK_LOG" ]]
run_case 1
[[ ! -e "$MOCK_LOG" ]]
run_case 0 --yes
grep -Fxq TEST_PHONE "$MOCK_LOG"
grep -Fxq testwan0 "$MOCK_LOG"
grep -Fxq NetworkManager.service "$MOCK_LOG"
grep -Fq 'Setup complete.' "$scratch/output"
MOCK_INSTALL_STATUS=7 run_case 7 --yes
grep -Fq 'Files may already be installed.' "$scratch/output"
if grep -Fq 'Setup complete.' "$scratch/output"; then exit 1; fi
PHONE_SERIAL=$'invalid\nvalue' run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
WAN_INTERFACE='bad interface' run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
ADB_BIN=/nonexistent run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
READY_UNITS='bad;service' run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
READY_UNITS=$'NetworkManager.service\nUSB_CYCLE_ENABLED=1' run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
PHONE_SERIAL='' run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
MOCK_UID=0 run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
run_case 1 --unknown
[[ ! -e "$MOCK_LOG" ]]
# The preview judges the USB function set read-only, matching boot recovery.
run_case 0 --dry-run
grep -Fq 'Phone USB state:   not read (phone not authorized' "$scratch/output"
MOCK_DISCOVERY=pinned MOCK_USB_STATE=hisuite,mtp,mass_storage,adb run_case 0 --dry-run
grep -Fq 'Phone USB state:   off (hisuite,mtp,mass_storage,adb); an unhealthy boot WAN receives one enable request' "$scratch/output"
MOCK_DISCOVERY=pinned MOCK_USB_STATE=rndis,adb run_case 0 --dry-run
grep -Fq 'Phone USB state:   on (rndis,adb)' "$scratch/output"
MOCK_DISCOVERY=pinned MOCK_USB_STATE=hisuite,mtp,mass_storage run_case 0 --dry-run
grep -Fq 'no ADB function' "$scratch/output"
# shellcheck disable=SC2016
MOCK_DISCOVERY=pinned MOCK_USB_STATE='$(reboot)' run_case 0 --dry-run
grep -Fq 'not recognized (malformed or empty)' "$scratch/output"
MOCK_DISCOVERY=pinned run_case 0 --dry-run
grep -Fq 'not read (USB state query failed)' "$scratch/output"
# Another phone's transport is never queried.
MOCK_DISCOVERY=one MOCK_USB_STATE=rndis,adb run_case 0 --dry-run
grep -Fq 'not read (phone not authorized' "$scratch/output"
# Saved values work even when USB networking is absent. Explicit settings win.
cat > "$INSTALL_CONFIG_FILE" <<CONFIG
PHONE_SERIAL=SAVED_PHONE
WAN_INTERFACE=savedwan0
ADB_BIN=$ADB_BIN
READY_UNITS="NetworkManager.service saved-dhcp.service"
CONFIG
PHONE_SERIAL='' WAN_INTERFACE='' READY_UNITS='' run_case 0 --yes
grep -Fxq SAVED_PHONE "$MOCK_LOG"
grep -Fxq savedwan0 "$MOCK_LOG"
grep -Fxq 'NetworkManager.service saved-dhcp.service' "$MOCK_LOG"
run_case 0 --yes
grep -Fxq TEST_PHONE "$MOCK_LOG"
rm "$INSTALL_CONFIG_FILE"
# Fake sysfs: associate the USB network interface with the chosen phone serial.
mkdir -p "$scratch/usb/function" "$scratch/drivers/rndis_host" "$INSTALL_SYS_CLASS_NET/autowan0"
printf 'AUTO_PHONE\n' > "$scratch/usb/serial"
ln -s "$scratch/drivers/rndis_host" "$scratch/usb/function/driver"
ln -s "$scratch/usb/function" "$INSTALL_SYS_CLASS_NET/autowan0/device"
PHONE_SERIAL='' WAN_INTERFACE='' READY_UNITS='' MOCK_DISCOVERY=one MOCK_SERVICE=loaded run_case 0 --yes
grep -Fxq AUTO_PHONE "$MOCK_LOG"
grep -Fxq autowan0 "$MOCK_LOG"
grep -Fxq 'NetworkManager.service pc101-technitium.service' "$MOCK_LOG"
PHONE_SERIAL='' WAN_INTERFACE='' READY_UNITS='' MOCK_DISCOVERY=one run_case 0 --dry-run
[[ ! -e "$MOCK_LOG" ]]
grep -Fq 'Tip: Run: ip -brief link' "$scratch/output"
for scenario in multiple unauthorized offline tcp failed; do
    PHONE_SERIAL='' MOCK_DISCOVERY=$scenario run_case 1 --yes
    [[ ! -e "$MOCK_LOG" ]]
done
# Do not guess an interface belonging to a different USB phone.
WAN_INTERFACE='' run_case 1 --yes
[[ ! -e "$MOCK_LOG" ]]
# Saved text is never evaluated as shell code.
# shellcheck disable=SC2016
printf 'PHONE_SERIAL=$(touch %s)\n' "$scratch/injected" > "$INSTALL_CONFIG_FILE"
PHONE_SERIAL='' run_case 1 --yes
[[ ! -e "$scratch/injected" && ! -e "$MOCK_LOG" ]]
printf 'Install entry point regression tests passed.\n'
