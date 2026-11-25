# `vm selftest`: checks a running VM against what WN01 promises.
# Sourced by scripts/windows-vm/vm.
# shellcheck shell=bash

# Addresses other machines could use to reach the dev box.
external_addresses() {
    ip -o addr show scope global | awk '{ split($4, a, "/"); print a[1] }'
}

# SSH is forwarded on loopback only: no other address of the dev box has it.
check_ssh_loopback_only() {
    local listeners
    listeners="$(ss -ltnH "sport = :$SSH_PORT" | awk '{ print $4 }')"
    [[ -n "$listeners" ]] || die "nothing listens on port $SSH_PORT"
    if grep -v -x -F "127.0.0.1:$SSH_PORT" <<<"$listeners"; then
        die "SSH port $SSH_PORT listens beyond loopback"
    fi
    log "ok   SSH listens on 127.0.0.1:$SSH_PORT only"
}

# Opens two ports on the dev box for the network check to aim at: one on all
# addresses ($WILDCARD_PORT) and one on loopback only ($LOOPBACK_PORT). They
# stay open until the listener's input closes, which happens when this
# command ends, however it ends.
open_check_ports() {
    coproc CHECK_PORTS {
        exec python3 -I -c '
import socket, sys
wildcard = socket.create_server(("0.0.0.0", 0))
loopback = socket.create_server(("127.0.0.1", 0))
print(wildcard.getsockname()[1], loopback.getsockname()[1], flush=True)
sys.stdin.read()
'
    }
    read -r -t 10 WILDCARD_PORT LOOPBACK_PORT <&"${CHECK_PORTS[0]}" &&
        [[ "$WILDCARD_PORT" =~ ^[0-9]+$ && "$LOOPBACK_PORT" =~ ^[0-9]+$ ]] ||
        die "could not open ports on the dev box to check the VM against"
}

# host:port pairs the VM must not reach: each external address of the dev
# box with the port listening on all addresses, the same port through QEMU's
# host alias, and the port that listens only on the dev box's loopback.
forbidden_targets() {
    local addrs addr
    # errexit does not reach into a command substitution: fail by hand.
    addrs="$(external_addresses)" || return 1
    for addr in $addrs; do
        [[ "$addr" == *:* ]] || echo "$addr:$WILDCARD_PORT"
    done
    echo "10.0.2.2:$WILDCARD_PORT"
    echo "10.0.2.2:$LOOPBACK_PORT"
}

check_network() {
    local targets
    open_check_ports
    targets="$(forbidden_targets)" || die "could not list the dev box's addresses"
    targets="$(paste -sd, - <<<"$targets")"
    log "checking what the VM reaches (must not: $targets)"
    vm_push 60 "$VM_DIR" "C:\\work" "$SCRIPT_DIR/guest/net-check.ps1"
    vm_ssh 300 "$VM_DIR" "C:\\work\\net-check.ps1 -Targets $targets; exit \$LASTEXITCODE" ||
        die "the VM reaches more than it should"
}

check_click() {
    log "running the click test in the desktop"
    vm_push 60 "$VM_DIR" "C:\\work" "$SCRIPT_DIR/guest/click-test.ps1"
    run_job "$VM_DIR" normal 60 'C:\work' \
        'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\work\click-test.ps1' ||
        die "the injected click did not reach the test window"
}

# The ids of the VM's ping processes, one per line.
ping_ids() {
    vm_ssh 60 "$VM_DIR" '@(Get-Process -Name PING -ErrorAction SilentlyContinue).Id' | tr -d '\r'
}

# A job that outlives its deadline is ended with everything it started: a
# ping in the background and one in front, both far longer than the deadline.
# Pings that ran before the job are not its own and are left out.
check_deadline() {
    local rc=0 before after left
    before="$(ping_ids)" || die "could not list the VM's processes"
    log "running a job past its 5 s deadline"
    run_job "$VM_DIR" normal 5 'C:\work' 'start /b ping -n 600 127.0.0.1 >NUL & ping -n 600 127.0.0.1 >NUL' >/dev/null 2>&1 || rc=$?
    ((rc == 124)) || die "a job past its deadline returned $rc, not 124"
    after="$(ping_ids)" || die "could not list the VM's processes"
    left="$(grep -cvxF -f <(echo "$before") <<<"$after" || true)"
    [[ "$left" == 0 ]] || die "$left ping process(es) outlived the job's deadline"
    log "ok   a job past its deadline was ended with its whole tree"
}

check_screen() {
    local shot="$RUN_DIR/selftest-screen.png"
    cmd_screen "$shot" >/dev/null
    log "ok   screen captured: $shot"
}

cmd_selftest() {
    take_lock 120
    require_running
    check_ssh_loopback_only
    check_click
    check_deadline
    check_screen
    check_network
    log "selftest passed"
}
