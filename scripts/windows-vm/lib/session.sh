# The commands a session uses on a built VM: start, stop, status, ssh, run,
# screen, key and reset. Sourced by scripts/windows-vm/vm.
# shellcheck shell=bash

# Waits up to $2 seconds until both runner tasks in the VM run (each writes
# its pid to `ready` when it starts).
wait_runners() {
    local dir="$1" deadline=$((SECONDS + $2)) check
    check='foreach ($q in "normal","high") { $p = [int](Get-Content "C:\wn01\queue\$q\ready" -ErrorAction Stop); Get-Process -Id $p -ErrorAction Stop | Out-Null }'
    until vm_ssh 60 "$dir" "$check" >/dev/null 2>&1; do
        ((SECONDS < deadline)) || die "the runner tasks did not start within $2 s"
        sleep 5
    done
}

# Hands a command to a runner queue in the desktop and returns its exit code.
run_job() {
    local keys="$1" queue="$2" seconds="$3" workdir="$4" command="$5" encoded
    encoded="$(printf '%s' "$command" | base64 -w0)"
    vm_ssh $((seconds + 120)) "$keys" \
        "C:\\wn01\\run-job.ps1 -CommandBase64 $encoded -TimeoutSeconds $seconds -Queue $queue -WorkDir '$workdir'; exit \$LASTEXITCODE"
}

require_vm() {
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
        log "already running; SSH on 127.0.0.1:$SSH_PORT"
        return 0
    fi
    prepare_run_dir
    install_proxy "$RUN_DIR"
    trap 'kill_machine "$RUN_DIR"' EXIT
    start_machine "$RUN_DIR" "$RUN_DIR/disk.qcow2"
    wait_ssh "$RUN_DIR" "$VM_DIR" 600
    wait_runners "$VM_DIR" 300
    trap - EXIT
    log "running; SSH on 127.0.0.1:$SSH_PORT, Windows $(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["build"])' "$VM_DIR/info.json")"
    warn_expiry
}

cmd_stop() {
    take_lock 30
    machine_running "$RUN_DIR" || return 0
    shutdown_machine "$RUN_DIR" "$VM_DIR" 180 || log "the VM was ended without a clean shutdown"
}

cmd_reset() {
    take_lock 30
    machine_running "$RUN_DIR" && die "the VM is running; stop it first (vm stop)"
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

cmd_ssh() {
    require_running
    vm_ssh "${WN01_SSH_TIMEOUT:-3600}" "$VM_DIR" "$@"
}

cmd_push() {
    (($# >= 1 && $# <= 2)) || die "usage: vm push LOCAL_FILE_OR_FOLDER [WINDOWS_FOLDER, default C:\\work]"
    require_running
    vm_push "${WN01_SSH_TIMEOUT:-3600}" "$VM_DIR" "${2:-C:\\work}" "$1"
}

cmd_run() {
    local queue=normal seconds=120 workdir='C:\work'
    while (($#)); do
        case "$1" in
            --elevated) queue=high ;;
            --timeout) seconds="$2"; shift ;;
            --workdir) workdir="$2"; shift ;;
            --) shift; break ;;
            *) break ;;
        esac
        shift
    done
    (($#)) || die "usage: vm run [--elevated] [--timeout SECONDS] [--workdir DIR] -- COMMAND..."
    [[ "$seconds" =~ ^[0-9]+$ ]] && ((seconds >= 1 && seconds <= 3600)) || die "--timeout takes 1 to 3600 seconds"
    [[ "$workdir" != *"'"* ]] || die "--workdir cannot contain a quote"
    take_lock "$seconds"
    require_running
    run_job "$VM_DIR" "$queue" "$seconds" "$workdir" "$*"
}

cmd_screen() {
    local out="${1:-$PWD/wn01-screen.png}"
    require_running
    out="$(realpath -m "$out")"
    rm -f "$out"
    monitor "$RUN_DIR" "screendump $out -f png" >/dev/null || die "the QEMU monitor did not answer"
    [[ "$(head -c 8 "$out" 2>/dev/null | od -An -tx1 | tr -d ' \n')" == 89504e470d0a1a0a ]] ||
        die "no screenshot was written to $out"
    echo "$out"
}

cmd_key() {
    (($#)) || die "usage: vm key KEYS... (QEMU names, e.g. ctrl-alt-delete, ret, meta_l-r)"
    require_running
    local key
    for key in "$@"; do
        [[ "$key" =~ ^[a-z0-9_-]+$ ]] || die "not a QEMU key name: $key"
        monitor "$RUN_DIR" "sendkey $key" >/dev/null || die "the QEMU monitor did not answer"
    done
}
