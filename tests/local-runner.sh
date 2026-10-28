#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/bin"
export TEST_LOG_ROOT="$scratch/records" RECOVERY_BIN="$scratch/bin/recover"
export PATH="$scratch/bin:$PATH" PHONE_SERIAL=TEST_PHONE WAN_INTERFACE=testwan0
cat > "$RECOVERY_BIN" <<'MOCK'
#!/usr/bin/env bash
printf 'simulated WAN offline\n'
if [[ -n ${MOCK_SLEEP:-} ]]; then sleep "$MOCK_SLEEP"; fi
exit "${MOCK_EXIT:-2}"
MOCK
for tool in adb ip nmcli; do
    cat > "$scratch/bin/$tool" <<'MOCK'
#!/usr/bin/env bash
printf 'local diagnostic: %s\n' "$*"
# Diagnostics can also fail when the phone/WAN is absent.
exit 1
MOCK
done
chmod +x "$RECOVERY_BIN" "$scratch/bin/adb" "$scratch/bin/ip" "$scratch/bin/nmcli"
output=$(./bin/adb-wan-test --check) && exit 1 || code=$?
[[ "$code" == 2 ]]
record=${output#Local test records: }
grep -Fxq 'status=failed' "$record/result"
grep -Fxq 'stage=complete' "$record/result"
grep -Fxq 'exit_code=2' "$record/result"
grep -Fq 'Recovery command exit: 2' "$record/output.log"
[[ $(grep -c 'Snapshot:' "$record/output.log") == 2 ]]
[[ $(stat -c %a "$record") == 700 ]]
[[ $(stat -c %a "$record/result") == 600 ]]
output=$(MOCK_EXIT=0 ./bin/adb-wan-test --check)
record=${output#Local test records: }
grep -Fxq 'status=passed' "$record/result"
grep -Fxq 'exit_code=0' "$record/result"
code=0
output=$(TEST_TIMEOUT=1 MOCK_SLEEP=5 ./bin/adb-wan-test --check) || code=$?
[[ "$code" == 124 ]]
record=${output#Local test records: }
grep -Fxq 'exit_code=124' "$record/result"
grep -Fxq 'stage=complete' "$record/result"
[[ $(grep -c 'Snapshot:' "$record/output.log") == 2 ]]
printf 'Local runner regression tests passed.\n'
