# Shared settings and helpers for scripts/windows-vm/vm. Sourced, not run.
# shellcheck shell=bash

STATE_DIR="${WN01_STATE_DIR:-$HOME/.local/share/auto-clicker-vm}"
# Paths under it go into QEMU's comma-separated options and the proxy's
# command line, which GLib splits as a shell would: only characters that mean
# nothing to either are allowed.
[[ "$STATE_DIR" =~ ^/[A-Za-z0-9._/+-]+$ ]] || {
    echo "vm: the state directory must be an absolute path of letters, digits and ._/+- only: '$STATE_DIR'" >&2
    exit 2
}
CACHE_DIR="$STATE_DIR/cache"
# The current VM: its clean base disk, firmware variables and TPM state, the
# SSH key and password, and what the build recorded (info.json).
VM_DIR="$STATE_DIR/vm"
# A started VM: the overlay on the base disk, copies of the variables and TPM
# state, sockets, pid files and logs. `vm reset` removes it.
RUN_DIR="$VM_DIR/run"
SSH_PORT="${WN01_SSH_PORT:-52222}"
# It goes into QEMU's comma-separated -netdev option: a port and nothing else.
[[ "$SSH_PORT" =~ ^[1-9][0-9]{0,4}$ ]] && ((SSH_PORT <= 65535)) || {
    echo "vm: WN01_SSH_PORT must be a port number, not '$SSH_PORT'" >&2
    exit 2
}
LOCK_FILE="$STATE_DIR/lock"

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

# The script's own stderr, kept as fd 8: an exit trap that runs while a
# command's stderr is sent elsewhere (`vm_ssh ... 2>/dev/null`) still reports.
exec 8>&2

log() {
    printf '%s vm: %s\n' "$(date -u +%H:%M:%S)" "$*" >&8
}

die() {
    log "error: $*"
    exit 1
}

# Runs a command, killing it after $1 seconds. --foreground keeps it in our
# process group, so Ctrl-C reaches it; 9>&- keeps it from holding the VM's
# lock should it outlive us, and 8>&- from holding our stderr open.
bounded() {
    local seconds="$1"
    shift
    timeout --foreground --kill-after=10 "$seconds" "$@" 8>&- 9>&-
}

# Checks that $2 is a whole number of seconds from 1 to $3, written in decimal
# without leading zeros (bash would read 010 as octal), and prints it.
seconds_arg() {
    local name="$1" value="$2" max="$3"
    [[ "$value" =~ ^[1-9][0-9]{0,5}$ ]] && ((value <= max)) || die "$name takes 1 to $max seconds, not '$value'"
    printf '%s' "$value"
}

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

# Takes the VM's lock for the rest of this process, waiting at most $1
# seconds: one session at a time builds, starts, stops, resets or runs a job.
# Daemons started under it close fd 9, or they would hold it after we exit.
take_lock() {
    mkdir -p "$STATE_DIR"
    chmod 700 "$STATE_DIR"
    exec 9>"$LOCK_FILE"
    flock -w "$1" 9 || die "another session is using the VM (lock held for over $1 s)"
}

# Keeps copy-on-write off for the disk images on btrfs, where it fragments them.
no_cow() {
    if [[ "$(stat -f -c %T "$1")" == btrfs ]]; then
        chattr +C "$1"
    fi
}

# Downloads $1 into the cache as $2 unless a verified copy is there, and
# refuses (deleting the download) when its SHA-256 is not $3.
fetch_verified() {
    local url="$1" name="$2" sha="$3" path="$CACHE_DIR/$2"
    mkdir -p "$CACHE_DIR"
    if [[ ! -f "$path" ]]; then
        log "downloading $name"
        bounded 3600 curl --fail --location --silent --show-error --retry 3 --continue-at - \
            --speed-limit 100000 --speed-time 120 --output "$path.part" "$url" ||
            die "download of $name failed"
        mv "$path.part" "$path"
    fi
    if ! echo "$sha  $path" | sha256sum --check --status; then
        rm -f "$path"
        die "$name does not match its pinned SHA-256; deleted it"
    fi
}
