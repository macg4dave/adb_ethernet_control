#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin"
export PATH="$scratch/bin:$PATH" MOCK_LOG="$scratch/log" MOCK_CONFIG="$scratch/config"
export WAN_INTERFACE=testwan0 WAN_CONNECTION_UUID=11111111-2222-3333-4444-555555555555
export PHONE_SERIAL=TEST_PHONE ADB_BIN="$scratch/bin/adb"
printf "#!/usr/bin/env bash\nexit 0\n" > "$scratch/bin/adb"
cat > "$scratch/bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
if [[ "$*" == *SubState* ]]; then echo dead; else echo enabled; fi
MOCK
cat > "$scratch/bin/journalctl" <<'MOCK'
#!/usr/bin/env bash
exit 0
MOCK
cat > "$scratch/bin/restorecon" <<'MOCK'
#!/usr/bin/env bash
exit 99
MOCK
cat > "$scratch/bin/sudo" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$MOCK_LOG"
if [[ "$1" == install && "${*: -1}" == /etc/adb-ethernet-control/boot.env ]]; then
    cp -- "${@: -2:1}" "$MOCK_CONFIG"
fi
if [[ "$*" == 'systemctl restart adb-wan-recover.service' && "${MOCK_FAIL_START:-0}" == 1 ]]; then exit 1; fi
MOCK
chmod +x "$scratch/bin/"*
./scripts/install-boot-service > "$scratch/output"
grep -Fxq 'WAN_INTERFACE=testwan0' "$MOCK_CONFIG"
grep -Fxq "PHONE_SERIAL=$PHONE_SERIAL" "$MOCK_CONFIG"
grep -Fxq 'USB_CYCLE_ENABLED=0' "$MOCK_CONFIG"
grep -Fxq 'POST_ENABLE_DELAY=20' "$MOCK_CONFIG"
grep -Fxq 'CONNECTION_TIMEOUT=45' "$MOCK_CONFIG"
awk '/systemd-analyze verify/ { verified=1 } /systemctl stop adb-wan-recover.service adb-wan-server.service/ { if (!verified) exit 1; stopped=1 } /systemctl restart/ { if (!stopped) exit 1; started=1 } /systemctl enable/ { if (!started) exit 1; enabled=1 } END { if (!enabled) exit 1 }' "$MOCK_LOG"
: > "$MOCK_LOG"
code=0
MOCK_FAIL_START=1 ./scripts/install-boot-service > "$scratch/output" 2>&1 || code=$?
[[ "$code" == 1 ]]
if grep -q 'systemctl enable' "$MOCK_LOG"; then exit 1; fi
printf 'Boot installer regression tests passed.\n'
