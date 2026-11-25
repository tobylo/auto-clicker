# Starting, reaching and stopping one QEMU machine whose files live in a
# directory (a build's temp directory, or the VM's run directory).
# Sourced by scripts/windows-vm/vm, after lib/common.sh.
# shellcheck shell=bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The process in pid file $1 if it is alive, is QEMU or swtpm, and its
# command line names $2, so a recycled pid is never taken for ours.
live_pid() {
    local pidfile="$1" marker="$2" pid exe
    [[ -f "$pidfile" ]] || return 1
    pid="$(<"$pidfile")"
    [[ "$pid" =~ ^[0-9]+$ ]] || return 1
    exe="$(readlink "/proc/$pid/exe" 2>/dev/null)" || return 1
    [[ "${exe##*/}" == qemu-system-x86_64 || "${exe##*/}" == swtpm ]] || return 1
    tr '\0' ' ' <"/proc/$pid/cmdline" 2>/dev/null | grep -qF -- "$marker" || return 1
    echo "$pid"
}

machine_running() {
    live_pid "$1/qemu.pid" "$1/" >/dev/null
}

# Copies the proxy and its allowlists into the machine's directory, so a
# running VM never depends on the checkout that started it. Any other list
# there goes first: the proxy loads every allow-*.txt in the directory.
install_proxy() {
    local dir="$1"
    shift
    rm -f "$dir"/allow-*.txt
    cp "$SCRIPT_DIR/proxy.py" "$SCRIPT_DIR/allow-run.txt" "$dir/"
    local list
    for list in "$@"; do
        cp "$SCRIPT_DIR/$list" "$dir/"
    done
}

# The QEMU command line for the machine in $1 on disk $2. The network is
# user-mode with restrict=on: the VM reaches nothing but the proxy, which QEMU
# starts per connection, and the dev box reaches its SSH on loopback only.
qemu_command() {
    local dir="$1" disk="$2" lists proxy
    lists=("$dir"/allow-*.txt)
    proxy="/usr/bin/python3 -I $dir/proxy.py $dir/proxy.log ${lists[*]}"
    QEMU=(qemu-system-x86_64 -name wn01
        -machine q35,smm=on,accel=kvm
        -cpu host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time,hv_vpindex,hv_synic,hv_stimer,hv_frequencies
        -smp 8,sockets=1,cores=8,threads=1 -m 12G
        -rtc base=utc
        -global driver=cfi.pflash01,property=secure,value=on
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$OVMF_CODE"
        -drive "if=pflash,format=raw,unit=1,file=$dir/vars.fd"
        -chardev "socket,id=chrtpm,path=$dir/swtpm.sock"
        -tpmdev emulator,id=tpm0,chardev=chrtpm -device tpm-tis,tpmdev=tpm0
        -drive "if=none,id=disk,format=qcow2,discard=unmap,file=$disk"
        -device ide-hd,bus=ide.0,drive=disk,bootindex=1
        -netdev "user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:$SSH_PORT-:22,guestfwd=tcp:10.0.2.100:3128-cmd:$proxy"
        -device e1000e,netdev=n0
        -device qemu-xhci,id=xhci -device usb-tablet,bus=xhci.0
        -vga none -device VGA,xres=1920,yres=1080,edid=on
        -display none
        -monitor "unix:$dir/monitor.sock,server=on,wait=off"
        -daemonize -pidfile "$dir/qemu.pid")
}

start_tpm() {
    local dir="$1"
    mkdir -p "$dir/tpm"
    swtpm socket --tpm2 --tpmstate "dir=$dir/tpm" \
        --ctrl "type=unixio,path=$dir/swtpm.sock" \
        --log "file=$dir/swtpm.log" --pid "file=$dir/swtpm.pid" \
        --terminate --daemon 9>&- || die "swtpm did not start"
}

# Starts the machine in $1 on disk $2, with extra QEMU arguments after them.
start_machine() {
    local dir="$1" disk="$2"
    shift 2
    machine_running "$dir" && die "the machine in $dir is already running"
    rm -f "$dir/monitor.sock" "$dir/swtpm.sock"
    start_tpm "$dir"
    qemu_command "$dir" "$disk"
    # 9>&-: the daemons must not inherit the VM's lock (lib/common.sh).
    bounded 60 "${QEMU[@]}" "$@" 2>>"$dir/qemu.log" 9>&- || die "QEMU did not start; see $dir/qemu.log"
}

# Sends one human-monitor command to the machine in $1 and prints its answer.
monitor() {
    local dir="$1"
    shift
    printf '%s\n' "$*" | bounded 30 socat -t 10 - "UNIX-CONNECT:$dir/monitor.sock"
}

# Waits up to $2 seconds for the machine in $1 to exit.
wait_exit() {
    local dir="$1" deadline=$((SECONDS + $2))
    while machine_running "$dir"; do
        ((SECONDS < deadline)) || return 1
        sleep 2
    done
}

# Ends the machine in $1 at once (no guest shutdown) and its TPM with it.
# Fails, keeping its pid files and sockets so a later call can try again,
# when QEMU is still alive after SIGKILL.
kill_machine() {
    local dir="$1" pid
    if machine_running "$dir"; then
        monitor "$dir" quit >/dev/null 2>&1 || true
        if ! wait_exit "$dir" 15; then
            pid="$(live_pid "$dir/qemu.pid" "$dir/")" && { kill -KILL "$pid" 2>/dev/null || true; }
            if ! wait_exit "$dir" 15; then
                log "QEMU in $dir did not exit, even after SIGKILL"
                return 1
            fi
        fi
    fi
    # It may have ended on its own between the check and the kill.
    pid="$(live_pid "$dir/swtpm.pid" "$dir/")" && { kill -TERM "$pid" 2>/dev/null || true; }
    rm -f "$dir/qemu.pid" "$dir/swtpm.pid" "$dir/monitor.sock" "$dir/swtpm.sock"
}

# SSH to tester in the VM whose keys are in $2, ended after $1 seconds.
# HOSTKEY=accept-new only while a build first meets the VM; afterwards the
# recorded key must match. (A function, so callers pass the deadline here:
# `bounded` cannot run a shell function.)
vm_ssh() {
    local seconds="$1" keys="$2"
    shift 2
    bounded "$seconds" ssh -F /dev/null -p "$SSH_PORT" -i "$keys/key" -o IdentitiesOnly=yes \
        -o UserKnownHostsFile="$keys/known_hosts" -o StrictHostKeyChecking="${HOSTKEY:-yes}" \
        -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 \
        -o LogLevel=ERROR tester@127.0.0.1 "$@"
}

# Copies local file or folder $4 into folder $3 in the VM whose keys are in
# $2, within $1 seconds, as a tar stream over SSH: Windows' OpenSSH server
# ends its SFTP subsystem at once, so scp and sftp do not work.
vm_push() {
    local seconds="$1" keys="$2" dest="$3" path="$4"
    [[ "$dest" =~ ^[A-Za-z]:[A-Za-z0-9_.\\-]*$ ]] || die "use a Windows folder without spaces or quotes: $dest"
    [[ -e "$path" ]] || die "no such file or folder: $path"
    tar -c -f - -C "$(dirname "$path")" "$(basename "$path")" |
        vm_ssh "$seconds" "$keys" "New-Item -ItemType Directory -Force $dest | Out-Null; cmd /c 'tar -x -f - -C $dest'; exit \$LASTEXITCODE"
}

# Waits up to $3 seconds for SSH into the machine in $1 (keys in $2) to answer,
# failing at once if the machine exits.
wait_ssh() {
    local dir="$1" keys="$2" deadline=$((SECONDS + $3))
    until vm_ssh 30 "$keys" 'exit 0' >/dev/null 2>&1; do
        machine_running "$dir" || die "the VM exited while waiting for SSH; see $dir/qemu.log"
        ((SECONDS < deadline)) || die "SSH did not answer within $3 s"
        sleep 10
    done
}

# Shuts Windows down cleanly, within $3 seconds, else ends the machine.
shutdown_machine() {
    local dir="$1" keys="$2" seconds="$3"
    machine_running "$dir" || return 0
    vm_ssh 30 "$keys" 'Stop-Computer -Force' >/dev/null 2>&1 ||
        monitor "$dir" system_powerdown >/dev/null 2>&1 || true
    if wait_exit "$dir" "$seconds"; then
        kill_machine "$dir" || die "the VM in $dir could not be ended"
        return 0
    fi
    log "Windows did not shut down within $seconds s; ending the VM"
    kill_machine "$dir" || die "the VM in $dir could not be ended"
    return 1
}
