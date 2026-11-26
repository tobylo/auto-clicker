# The VM's settings and helpers for scripts/windows-vm/vm. Sourced, not run.
# shellcheck shell=bash

TOOL_NAME=vm
MACHINE_NAME="the VM"
# shellcheck source=base.sh
source "$(dirname "${BASH_SOURCE[0]}")/base.sh"

# Running processes are recognised by the spelling of their directory, so
# every session uses the one physical path, which set_state_dir resolves.
# Paths under it go into QEMU's comma-separated options and the proxy's
# command line, which GLib splits as a shell would: set_state_dir allows only
# characters that mean nothing to either.
set_state_dir "${WN01_STATE_DIR:-$HOME/.local/share/auto-clicker-vm}"
CACHE_DIR="$STATE_DIR/cache"
# The current VM: its clean base disk, firmware variables and TPM state, the
# SSH key and password, and what the build recorded (info.json).
VM_DIR="$STATE_DIR/vm"
# A started VM: the overlay on the base disk, copies of the variables and TPM
# state, sockets and logs. `vm reset` removes it.
RUN_DIR="$VM_DIR/run"
# lib/remote.sh's commands reach the VM with its key and known_hosts, on the
# loopback forward, never where the caller's environment points.
KEYS_DIR="$VM_DIR"
SSH_HOST=127.0.0.1
SSH_KEY_FILE=""
SSH_HOST_ALIAS=""
REMOTE_PREFIX=""
RUN_JOB_PREFIX=""
SSH_PORT="${WN01_SSH_PORT:-52222}"
# It goes into QEMU's comma-separated -netdev option: a port and nothing else.
[[ "$SSH_PORT" =~ ^[1-9][0-9]{0,4}$ ]] && ((SSH_PORT <= 65535)) || {
    echo "vm: WN01_SSH_PORT must be a port number, not '$SSH_PORT'" >&2
    exit 2
}

# Microsoft's Windows 11 Enterprise evaluation, 26H2, en-US, x64 (build
# 26300.9457), and its SHA-256 as Microsoft publishes it at
# https://aka.ms/VerifyWindowsISO. When Microsoft replaces the image, the
# build refuses the new one until both lines are updated together.
ISO_URL="https://go.microsoft.com/fwlink/?LinkId=2382600&clcid=0x409&culture=en-us&country=us"
ISO_SHA256="bc3f24086ebadc94489066b5ad78089e2cf5c3491e90e790bb81a2b199c10e38"
ISO_FILE="win11-enterprise-eval-26h2-x64-en-us.iso"

# Microsoft's signed OpenSSH for Windows, from its PowerShell/Win32-OpenSSH
# releases; the SHA-256 is the one GitHub lists for the asset.
OPENSSH_URL="https://github.com/PowerShell/Win32-OpenSSH/releases/download/10.4.0.0p1-Preview/OpenSSH-Win64-v10.4.0.0.msi"
OPENSSH_SHA256="788d8643464a01eb63640a2fd746f9ed23d106b5a901a35f88dc8cd40e446ac9"
OPENSSH_FILE="OpenSSH-Win64-v10.4.0.0.msi"

OVMF_CODE=/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd
OVMF_VARS=/usr/share/edk2/x64/OVMF_VARS.4m.fd

require_tools() {
    local tool missing=()
    for tool in qemu-system-x86_64 qemu-img swtpm socat mformat mcopy ssh ssh-keygen tar \
        python3 curl flock sha256sum timeout ss; do
        command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
    done
    [[ -r "$OVMF_CODE" && -r "$OVMF_VARS" ]] || missing+=("OVMF ($OVMF_CODE)")
    # QEMU starts the proxy by this path, not through PATH.
    [[ -x /usr/bin/python3 ]] || missing+=("/usr/bin/python3")
    [[ -w /dev/kvm ]] || missing+=("/dev/kvm access")
    ((${#missing[@]} == 0)) || die "missing: ${missing[*]}"
}

# Keeps copy-on-write off for the disk images on btrfs, where it fragments them.
no_cow() {
    if [[ "$(stat -f -c %T "$1")" == btrfs ]]; then
        chattr +C "$1"
    fi
}

# The size in bytes of partial download $1, 0 when there is none.
part_size() {
    if [[ -f "$1" ]]; then stat -c %s -- "$1"; else echo 0; fi
}

# Downloads $1 into the cache as $2 unless a verified copy is there, and
# refuses (deleting the download) when its SHA-256 is not $3.
fetch_verified() {
    local url="$1" name="$2" sha="$3" path="$CACHE_DIR/$2" rc=0 had
    mkdir -p "$CACHE_DIR"
    if [[ ! -f "$path" ]]; then
        log "downloading $name"
        had="$(part_size "$path.part")"
        bounded 3600 curl --fail --location --silent --show-error --retry 3 --continue-at - \
            --speed-limit 100000 --speed-time 120 --output "$path.part" "$url" || rc=$?
        if ((rc != 0)); then
            # A resume that got nowhere (the server refused the range, or
            # answered 416) would fail the same way again: start over next time.
            (($(part_size "$path.part") > had)) || rm -f "$path.part"
            die "download of $name failed (curl exit $rc)"
        fi
        mv "$path.part" "$path"
    fi
    if ! echo "$sha  $path" | sha256sum --check --status; then
        rm -f "$path"
        die "$name does not match its pinned SHA-256; deleted it"
    fi
}
