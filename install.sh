#!/usr/bin/env bash
# Friendly entry point; privileged installation stays in scripts/install-boot-service.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
assume_yes=false
dry_run=false
usage() {
    cat <<'HELP'
Usage: ./install.sh [--yes] [--dry-run]

Install, check and enable phone WAN boot recovery. Run as your normal login
user from a local terminal; sudo will ask for your password during installation.

Options:
  --yes       Accept supplied or unambiguous detected settings without prompts.
  --dry-run   Show settings without installing or changing networking.
  -h, --help  Show this help.

Settings: PHONE_SERIAL, WAN_INTERFACE, ADB_BIN, READY_UNITS.
Example:
  PHONE_SERIAL=YOUR_USB_SERIAL WAN_INTERFACE=enp48s0f4u2 ./install.sh --yes
HELP
}
error() { printf 'Error: %s\n' "$*" >&2; exit 1; }
for arg in "$@"; do
    case "$arg" in
        --yes) assume_yes=true ;;
        --dry-run) dry_run=true ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; error "Unknown option: $arg" ;;
    esac
done
[[ $(id -u) != 0 ]] || error 'Run ./install.sh as your normal user, without sudo, so ADB uses your existing phone authorization.'
bold='' reset=''
if [[ -t 1 && -z ${NO_COLOR:-} && ${TERM:-dumb} != dumb ]]; then
    bold=$'\033[1;36m'; reset=$'\033[0m'
fi
printf '\n%sADB Ethernet Control — boot recovery setup%s\n\n' "$bold" "$reset"
interactive=false
if [[ -t 0 ]] && ! "$assume_yes"; then interactive=true; fi
prompt() {
    local name=$1 label=$2 tip=$3 value
    printf '\n%s\n  Tip: %s\n' "$label" "$tip"
    if "$interactive"; then
        printf '  Edit the suggested answer, or press Enter to accept it.\n'
        read -r -e -i "${!name:-}" -p '  > ' value || error 'Input closed; installation canceled.'
        printf -v "$name" '%s' "$value"
    else
        printf '  Suggested answer: %s\n' "${!name:-(not found)}"
    fi
    [[ -n ${!name:-} ]] || error "Set $name or run interactively in a terminal. See ./install.sh --help."
}
# Read only the four supported settings as data, never source an environment file.
# Explicit environment settings take precedence over an existing installation.
config_file=${INSTALL_CONFIG_FILE:-/etc/adb-ethernet-control/boot.env}
if [[ -r "$config_file" ]]; then
    while IFS='=' read -r key value || [[ -n "$key" ]]; do
        case "$key" in PHONE_SERIAL|WAN_INTERFACE|ADB_BIN|READY_UNITS) ;; *) continue ;; esac
        if [[ "$value" == \"*\" ]]; then value=${value:1:${#value}-2}; fi
        if [[ -z ${!key:-} ]]; then printf -v "$key" '%s' "$value"; fi
    done < "$config_file"
    printf 'Using readable settings from the existing installation as defaults.\n'
fi
PHONE_SERIAL=${PHONE_SERIAL:-}
WAN_INTERFACE=${WAN_INTERFACE:-}
ADB_BIN=${ADB_BIN:-$(command -v adb || true)}
READY_UNITS=${READY_UNITS:-}
command -v timeout >/dev/null || error 'The timeout command is required for bounded discovery.'
prompt ADB_BIN 'ADB executable' 'Run: command -v adb. Use the absolute path to your installed adb executable.'
[[ "$ADB_BIN" =~ ^/[a-zA-Z0-9/._-]+$ && -x "$ADB_BIN" ]] || error 'ADB_BIN must be an executable absolute path without spaces.'
if [[ -z "$PHONE_SERIAL" ]]; then
    # Explicit hosts prevent discovery from starting or replacing an ADB server.
    for port in 5038 5037; do
        listing=$(timeout --kill-after=2s 3s "$ADB_BIN" -H 127.0.0.1 -P "$port" devices -l 2>/dev/null) || continue
        candidates=()
        authorized=()
        while read -r serial state details; do
            [[ "$serial" =~ ^[a-zA-Z0-9._-]+$ && " $details " == *' usb:'* ]] || continue
            candidates+=("$serial")
            printf '  USB phone candidate: %s (%s), ADB port %s\n' "$serial" "$state" "$port"
            if [[ "$state" == device ]]; then authorized+=("$serial"); fi
        done <<< "$listing"
        if ((${#candidates[@]})); then
            if ((${#candidates[@]} == 1 && ${#authorized[@]} == 1)); then PHONE_SERIAL=${authorized[0]}; fi
            break
        fi
    done
    if [[ -z "$PHONE_SERIAL" ]]; then printf 'No single authorized USB phone found; enter its serial manually.\n'; fi
fi
printf 'Use the phone USB serial (not a TCP address) and its host WAN interface.\n'
prompt PHONE_SERIAL 'Authorized phone USB serial' 'Run adb devices -l with your existing server; look for a device with usb:. Unlock the phone and accept USB debugging authorization if needed.'
[[ "$PHONE_SERIAL" =~ ^[a-zA-Z0-9._:-]+$ ]] || error 'Invalid phone serial.'
if [[ -z "$WAN_INTERFACE" ]]; then
    matched=() networking=()
    for entry in "${INSTALL_SYS_CLASS_NET:-/sys/class/net}"/*; do
        [[ -e "$entry/device" ]] || continue
        device_path=$(readlink -f -- "$entry/device") || continue
        driver=$(readlink -f -- "$entry/device/driver") || continue
        case "${driver##*/}" in rndis_host|cdc_ncm|cdc_ether) ;; *) continue ;; esac
        interface=${entry##*/}
        printf '  USB network candidate: %s (%s)\n' "$interface" "${driver##*/}"
        usb_serial=''
        while [[ "$device_path" != / && -n "$device_path" ]]; do
            if [[ -r "$device_path/serial" ]]; then
                IFS= read -r usb_serial < "$device_path/serial" || true
                if [[ "$usb_serial" == "$PHONE_SERIAL" ]]; then matched+=("$interface"); fi
                break
            fi
            device_path=${device_path%/*}
        done
        if [[ -z "$usb_serial" && ( "${driver##*/}" == rndis_host || "${driver##*/}" == cdc_ncm ) ]]; then
            networking+=("$interface")
        fi
    done
    if ((${#matched[@]} == 1)); then WAN_INTERFACE=${matched[0]}
    elif ((${#matched[@]} == 0 && ${#networking[@]} == 1)); then WAN_INTERFACE=${networking[0]}; fi
fi
prompt WAN_INTERFACE 'Phone WAN interface' 'Run: ip -brief link or nmcli device status. Choose the phone USB interface, not a LAN bridge. If USB Ethernet is off, use its saved NetworkManager profile interface name.'
[[ "$WAN_INTERFACE" =~ ^[a-zA-Z0-9._:-]+$ ]] || error 'Invalid WAN interface.'
if [[ -z "$READY_UNITS" ]]; then
    READY_UNITS=NetworkManager.service
    if [[ $(timeout --kill-after=2s 3s systemctl show pc101-technitium.service -p LoadState --value 2>/dev/null) == loaded ]]; then
        READY_UNITS+=' pc101-technitium.service'
    fi
fi
prompt READY_UNITS 'Services to wait for at boot' 'Run: systemctl list-units --type=service. Include NetworkManager.service and the actual LAN DNS/DHCP service, separated by spaces.'
[[ "$READY_UNITS" != *$'\n'* && "$READY_UNITS" != *$'\r'* ]] || error 'Readiness services must be on one line.'
read -r -a units <<< "$READY_UNITS"
((${#units[@]})) || error 'Choose at least one readiness service.'
for unit in "${units[@]}"; do
    [[ "$unit" =~ ^[a-zA-Z0-9_.@:-]+\.service$ ]] || error "Invalid readiness service: $unit"
done
printf '\n%sInstallation settings%s\n' "$bold" "$reset"
printf '  Phone USB serial:  %s\n  WAN interface:     %s\n  ADB executable:    %s\n  Wait for services: %s\n' \
    "$PHONE_SERIAL" "$WAN_INTERFACE" "$ADB_BIN" "$READY_UNITS"
printf '  WAN check:         ping 8.8.8.8 through the phone interface\n'
printf '  Connection wait:   20 seconds settling + up to 45 seconds for IPv4\n'
printf '  Automatic cycling: disabled until independent restoration is verified\n\n'
printf 'Installs system services and USB access rules, runs recovery now, and\n'
printf 'enables boot recovery after a successful check. An unhealthy WAN may\n'
printf 'receive one USB enable request. Local logs: /var/log/adb-wan-boot/\n\n'
if "$dry_run"; then printf 'Preview complete. No installation performed.\n'; exit 0; fi
if "$interactive"; then
    read -r -p 'Install with these settings? [Y/n] ' answer || error 'Input closed; installation canceled.'
    case "$answer" in ''|y|Y|yes|YES) ;; *) printf 'Installation canceled.\n'; exit 0 ;; esac
elif ! "$assume_yes"; then
    error 'Use --yes for an unattended install, or --dry-run to preview settings.'
fi
for tool in sudo systemctl systemd-analyze timeout ping ip flock udevadm; do
    command -v "$tool" >/dev/null || error "Required command missing: $tool. Install it before rerunning setup."
done
export PHONE_SERIAL WAN_INTERFACE ADB_BIN READY_UNITS
printf '%sInstalling and checking boot recovery…%s\n' "$bold" "$reset"
if "$repo/scripts/install-boot-service"; then
    printf '\n%sSetup complete.%s\n' "$bold" "$reset"
    printf 'View status: systemctl status adb-wan-recover.service --no-pager\n'
else
    code=$?
    printf '\nInstallation or its recovery check failed (exit %s).\n' "$code" >&2
    printf 'Files may already be installed. Inspect: journalctl -u adb-wan-recover.service --no-pager\n' >&2
    exit "$code"
fi
