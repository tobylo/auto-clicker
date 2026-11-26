# The commands a session uses on a built VM: start, stop, status, screen, key
# and reset (ssh, push and run are in lib/remote.sh). Sourced by
# scripts/windows-vm/vm.
# shellcheck shell=bash

# Seconds since 1970 at which the running Windows booted, from the VM whose
# keys are in $1.
boot_time() {
    vm_ssh 60 "$1" '([DateTimeOffset] (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).ToUnixTimeSeconds()' |
        tr -d '\r'
}

# Under the lock: puts back a VM an interrupted rebuild left aside first.
require_vm() {
    restore_previous_vm
    [[ -f "$VM_DIR/base.qcow2" ]] || die "no VM built yet; run: vm build"
}

require_running() {
    machine_running "$RUN_DIR" || die "the VM is not running; run: vm start"
}

# Days until the evaluation recorded at build time expires.
days_left() {
    local expires
    expires="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["expires"])' "$VM_DIR/info.json")"
    echo $((($(date -u -d "$expires" +%s) - $(date -u +%s)) / 86400))
}

warn_expiry() {
    local left
    left="$(days_left)"
    if ((left < 14)); then
        log "warning: the Windows evaluation expires in $left days; rebuild with: vm build"
    fi
}

# Makes the run directory from the clean base when it does not exist.
prepare_run_dir() {
    [[ -f "$RUN_DIR/disk.qcow2" ]] && return 0
    rm -rf "$RUN_DIR"
    mkdir -p "$RUN_DIR"
    no_cow "$RUN_DIR"
    qemu-img create -q -f qcow2 -b "$VM_DIR/base.qcow2" -F qcow2 "$RUN_DIR/disk.qcow2.tmp"
    cp "$VM_DIR/base-vars.fd" "$RUN_DIR/vars.fd"
    chmod u+w "$RUN_DIR/vars.fd"
    cp -r "$VM_DIR/base-tpm" "$RUN_DIR/tpm"
    mv "$RUN_DIR/disk.qcow2.tmp" "$RUN_DIR/disk.qcow2"
}

cmd_start() {
    require_tools
    take_lock 60
    require_vm
    if machine_running "$RUN_DIR"; then
        log "already running; waiting for SSH and the runners"
    else
        prepare_run_dir
        install_proxy "$RUN_DIR"
        trap 'kill_machine "$RUN_DIR"' EXIT
        start_machine "$RUN_DIR" "$RUN_DIR/disk.qcow2"
    fi
    wait_ssh "$RUN_DIR" "$VM_DIR" 600
    wait_runners "$VM_DIR" 300
    trap - EXIT
    log "running; SSH on 127.0.0.1:$SSH_PORT, Windows $(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["build"])' "$VM_DIR/info.json")"
    warn_expiry
}

cmd_stop() {
    take_lock 30
    shutdown_machine "$RUN_DIR" "$VM_DIR" 180 || log "the VM was ended without a clean shutdown"
}

cmd_reset() {
    take_lock 30
    # Puts back a VM an interrupted rebuild left aside, so its changes go too.
    restore_previous_vm
    machine_running "$RUN_DIR" && die "the VM is running; stop it first (vm stop)"
    kill_machine "$RUN_DIR" || die "the TPM in $RUN_DIR could not be ended"
    rm -rf "$RUN_DIR"
    log "reset: the next start boots the clean build"
}

cmd_status() {
    if [[ ! -f "$VM_DIR/info.json" ]]; then
        echo "no VM built (state: $STATE_DIR)"
        return 0
    fi
    echo "built:   $(<"$VM_DIR/info.json")"
    echo "expires in $(days_left) days"
    if machine_running "$RUN_DIR"; then
        echo "state:   running, SSH on 127.0.0.1:$SSH_PORT"
    else
        echo "state:   stopped"
    fi
    [[ -f "$RUN_DIR/disk.qcow2" ]] && echo "changes: kept since the last reset (vm reset discards them)"
    return 0
}

cmd_screen() {
    local out="${1:-wn01-screen.png}" shot
    require_running
    # QEMU writes into the run folder, whose path holds no space (lib/common.sh),
    # so the monitor reads one command. The rename then takes $out as given
    # and replaces whatever it names, a symlink included, never writing
    # through it.
    shot="$(mktemp "$RUN_DIR/screen.XXXXXX")"
    if monitor "$RUN_DIR" "screendump $shot -f png" >/dev/null &&
        [[ "$(head -c 8 "$shot" | od -An -tx1 | tr -d ' \n')" == 89504e470d0a1a0a ]] &&
        mv -fT -- "$shot" "$out"; then
        echo "$out"
        return 0
    fi
    rm -f "$shot"
    die "no screenshot was written to $out"
}

cmd_key() {
    (($#)) || die "usage: vm key KEYS... (QEMU names, e.g. ctrl-alt-delete, ret, meta_l-r)"
    take_lock 600
    require_running
    local key
    for key in "$@"; do
        [[ "$key" =~ ^[a-z0-9_-]+$ ]] || die "not a QEMU key name: $key"
        monitor "$RUN_DIR" "sendkey $key" >/dev/null || die "the QEMU monitor did not answer"
    done
}
