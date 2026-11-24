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

# host:port pairs the VM must not reach: each external address of the dev
# box with a port listening on all addresses, the same port through QEMU's
# host alias, and one port that listens only on the dev box's loopback.
forbidden_targets() {
    local wildcard loopback addr
    wildcard="$(ss -ltnH | awk '$4 ~ /^0\.0\.0\.0:/ { sub(/.*:/, "", $4); print $4; exit }')"
    loopback="$(ss -ltnH | awk '$4 ~ /^127\.0\.0\.1:/ { sub(/.*:/, "", $4); print $4; exit }')"
    for addr in $(external_addresses); do
        [[ "$addr" == *:* ]] || echo "$addr:${wildcard:-22}"
    done
    echo "10.0.2.2:${wildcard:-22}"
    echo "10.0.2.2:${loopback:-631}"
}

check_network() {
    local targets
    targets="$(forbidden_targets | paste -sd, -)"
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
    check_screen
    check_network
    log "selftest passed"
}
