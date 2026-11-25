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

# A TCP port that listens on the dev box at address $1 (0.0.0.0 or
# 127.0.0.1), other than the VM's own SSH forward. A check against a port
# nothing listens on would pass whatever the VM can reach, so none found
# fails the selftest.
listening_port() {
    local port
    port="$(ss -ltnH | awk -v a="$1" -v skip="$SSH_PORT" '{ n = split($4, p, ":"); if (p[1] == a && p[n] != skip) { print p[n]; exit } }')"
    [[ -n "$port" ]] || die "no port listens on $1 on the dev box to check the VM against"
    echo "$port"
}

# host:port pairs the VM must not reach: each external address of the dev
# box with a port listening on all addresses, the same port through QEMU's
# host alias, and one port that listens only on the dev box's loopback.
forbidden_targets() {
    local wildcard loopback addr
    wildcard="$(listening_port 0.0.0.0)"
    loopback="$(listening_port 127.0.0.1)"
    for addr in $(external_addresses); do
        [[ "$addr" == *:* ]] || echo "$addr:$wildcard"
    done
    echo "10.0.2.2:$wildcard"
    echo "10.0.2.2:$loopback"
}

check_network() {
    local targets
    targets="$(forbidden_targets)"
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

# A job that outlives its deadline is ended with everything it started: a
# ping in the background and one in front, both far longer than the deadline.
check_deadline() {
    local rc=0 left
    log "running a job past its 5 s deadline"
    run_job "$VM_DIR" normal 5 'C:\work' 'start /b ping -n 600 127.0.0.1 >NUL & ping -n 600 127.0.0.1 >NUL' >/dev/null 2>&1 || rc=$?
    ((rc == 124)) || die "a job past its deadline returned $rc, not 124"
    left="$(vm_ssh 60 "$VM_DIR" '@(Get-Process -Name PING -ErrorAction SilentlyContinue).Count' | tr -d '\r')" ||
        die "could not list the VM's processes"
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
