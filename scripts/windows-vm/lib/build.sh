# `vm build`: from Microsoft's image to a clean, provisioned VM, built in a
# temp directory beside the current VM and swapped in only on success.
# Sourced by scripts/windows-vm/vm, after lib/common.sh and lib/machine.sh.
# shellcheck shell=bash

BUILD_DIR=""

# Every exit before the swap removes the half-made VM: its QEMU, its TPM and
# its directory.
abandon_build() {
    [[ -n "$BUILD_DIR" && -d "$BUILD_DIR" ]] || return 0
    keep_failure_evidence "$BUILD_DIR"
    log "build failed; removing $BUILD_DIR (its logs and last screen are in $STATE_DIR/last-failed-build)"
    kill_machine "$BUILD_DIR"
    rm -rf "$BUILD_DIR"
}

# Keeps the logs and the last screen of a failed build; never its disk or secrets.
keep_failure_evidence() {
    local dir="$1" keep="$STATE_DIR/last-failed-build" file
    rm -rf "$keep"
    mkdir -p "$keep"
    if machine_running "$dir"; then
        monitor "$dir" "screendump $keep/screen.png -f png" >/dev/null 2>&1 || log "no screenshot of the failed build"
    fi
    for file in qemu.log swtpm.log proxy.log; do
        [[ -f "$dir/$file" ]] && cp "$dir/$file" "$keep/"
    done
    return 0
}

# Removes what a build killed outright (SIGKILL skips its trap) left behind.
# A build killed between swap_in's two renames left the previous VM as
# old.*: that one is put back, not removed.
remove_stale_builds() {
    local dir
    for dir in "$STATE_DIR"/old.*/; do
        if [[ -d "$dir" && ! -e "$VM_DIR" ]]; then
            log "restoring the previous VM from $dir"
            mv "${dir%/}" "$VM_DIR"
        fi
    done
    for dir in "$STATE_DIR"/build.*/ "$STATE_DIR"/old.*/; do
        [[ -d "$dir" ]] || continue
        log "removing a stale build: $dir"
        kill_machine "${dir%/}"
        rm -rf "$dir"
    done
}

make_secrets() {
    local dir="$1"
    ssh-keygen -q -t ed25519 -N '' -C wn01 -f "$dir/key"
    cp "$dir/key.pub" "$dir/authorized_keys"
    : >"$dir/known_hosts"
    # Windows' default policy accepts this; autologon keeps it readable in the
    # VM's registry, which is why tester is used for nothing but this VM.
    (umask 077 && python3 -I -c 'import secrets; print("Wn1-" + secrets.token_hex(16))' >"$dir/password")
}

# The FAT setup stick: autounattend.xml (with the password), guest/, the SSH
# public key and the OpenSSH installer. Windows Setup finds the answer file at
# the root of removable media.
make_stick() {
    local dir="$1" stick="$1/stick.img" password
    password="$(<"$dir/password")"
    sed "s/@PASSWORD@/$password/g" "$SCRIPT_DIR/autounattend.xml" >"$dir/autounattend.xml"
    truncate -s 256M "$stick"
    mformat -i "$stick" -F -v WN01SETUP ::
    mcopy -i "$stick" "$dir/autounattend.xml" ::/autounattend.xml
    mcopy -i "$stick" "$dir/authorized_keys" ::/authorized_keys
    mcopy -i "$stick" "$CACHE_DIR/$OPENSSH_FILE" ::/openssh.msi
    mcopy -i "$stick" -s "$SCRIPT_DIR/guest" ::/guest
    rm "$dir/autounattend.xml"
}

make_disks() {
    local dir="$1"
    qemu-img create -q -f qcow2 "$dir/disk.qcow2" 128G
    cp "$OVMF_VARS" "$dir/vars.fd"
}

# Starts Windows Setup and presses a key through the "Press any key to boot
# from CD or DVD" prompt. The key is `x`, which nothing in Setup's windows
# answers to: Enter or Space would press Setup's focused Cancel button.
start_setup() {
    local dir="$1" i
    install_proxy "$dir" allow-build.txt
    start_machine "$dir" "$dir/disk.qcow2" \
        -drive "if=none,id=cd,media=cdrom,readonly=on,file=$CACHE_DIR/$ISO_FILE" \
        -device ide-cd,bus=ide.1,drive=cd,bootindex=0 \
        -drive "if=none,id=stick,format=raw,file=$dir/stick.img" \
        -device usb-storage,bus=xhci.0,drive=stick,removable=on \
        -device pcie-root-port,id=openport,chassis=9,slot=9
    for i in $(seq 30); do
        monitor "$dir" sendkey x >/dev/null || die "the QEMU monitor stopped answering"
        sleep 1
    done
}

# Runs `provision.ps1 -Step $2` in the VM in $1 over SSH, within $3 seconds.
provision() {
    local dir="$1" step="$2" seconds="$3"
    log "provisioning: $step (deadline $seconds s)"
    vm_ssh "$seconds" "$dir" "C:\\wn01\\provision.ps1 -Step $step; exit \$LASTEXITCODE" ||
        die "provisioning step $step failed"
}

# The OpenGL compatibility pack comes from the Store through winget, which
# works only in tester's desktop session, so it runs through the runner. The
# Store's download service ignores the proxy, so for this step alone the VM
# gets a second network card on unrestricted user networking (the internet,
# and the LAN too), removed again before the build goes on.
install_opengl_pack() {
    local dir="$1" command
    log "provisioning: OpenGL compatibility pack (network open for this step)"
    run_job "$dir" high 120 'C:\work' 'winget settings --enable BypassCertificatePinningForMicrosoftStore' ||
        die "could not set winget's Store setting"
    open_network "$dir"
    sleep 30
    command='winget install --id 9NQPSL29BFFF --source msstore --silent --accept-package-agreements --accept-source-agreements --disable-interactivity'
    run_job "$dir" normal 1200 'C:\work' "$command" || die "the OpenGL compatibility pack did not install"
    close_network "$dir"
}

# Sends a monitor command and fails on an error in its answer.
monitor_ok() {
    local dir="$1" answer
    shift
    answer="$(monitor "$dir" "$@")" || die "the QEMU monitor did not answer: $*"
    if grep -qi "error" <<<"$answer"; then
        die "QEMU refused '$*': $answer"
    fi
}

open_network() {
    monitor_ok "$1" "netdev_add user,id=open,net=10.0.3.0/24"
    monitor_ok "$1" "device_add e1000e,netdev=open,id=opennic,bus=openport"
}

# Removes the open network card; fails the build unless QEMU confirms it is
# gone within a minute.
close_network() {
    local dir="$1" deadline=$((SECONDS + 60))
    monitor_ok "$dir" "device_del opennic"
    while monitor "$dir" "info pci" | grep -q opennic; do
        ((SECONDS < deadline)) || die "the open network card was not removed"
        sleep 2
    done
    monitor_ok "$dir" "netdev_del open"
    log "network closed again"
}

# Restarts Windows and waits for SSH and the runners again.
restart_windows() {
    local dir="$1"
    vm_ssh 30 "$dir" 'Restart-Computer -Force' >/dev/null 2>&1 || true
    sleep 60
    wait_ssh "$dir" "$dir" 1800
    wait_runners "$dir" 300
}

# Windows Update passes until nothing is left, a restart after each.
update_windows() {
    local dir="$1" pass state
    for pass in 1 2 3 4 5 6; do
        provision "$dir" update-start 120
        state="$(wait_update "$dir" 10800)"
        log "Windows Update pass $pass: $state"
        [[ "$state" == done ]] && return 0
        [[ "$state" == reboot ]] || die "Windows Update failed"
        restart_windows "$dir"
    done
    die "Windows Update still had updates after 6 passes"
}

# Waits up to $2 seconds for the running update pass to finish; prints its state.
wait_update() {
    local dir="$1" deadline=$((SECONDS + $2)) state
    while ((SECONDS < deadline)); do
        state="$(vm_ssh 120 "$dir" 'C:\wn01\provision.ps1 -Step update-status' | tr -d '\r')" ||
            state=unreachable
        [[ "$state" == running || "$state" == unreachable ]] || break
        sleep 60
    done
    echo "${state:-timeout}"
}

# Turns the provisioned disk, variables and TPM state into the clean base.
seal_base() {
    local dir="$1"
    vm_ssh 300 "$dir" 'C:\wn01\provision.ps1 -Step info' | tr -d '\r' >"$dir/info.json" ||
        die "could not read the Windows build and expiry"
    vm_ssh 120 "$dir" 'Remove-Item -Recurse -Force C:\wn01\downloads -ErrorAction SilentlyContinue' ||
        die "could not clear the downloads"
    shutdown_machine "$dir" "$dir" 600 || die "Windows did not shut down cleanly"
    mv "$dir/disk.qcow2" "$dir/base.qcow2"
    mv "$dir/vars.fd" "$dir/base-vars.fd"
    mv "$dir/tpm" "$dir/base-tpm"
    chmod a-w "$dir/base.qcow2" "$dir/base-vars.fd"
    rm -f "$dir/stick.img" "$dir/authorized_keys" "$dir"/allow-*.txt "$dir/proxy.py"
}

# Replaces the current VM with the finished build.
swap_in() {
    local dir="$1" old="$STATE_DIR/old.$$"
    if [[ -d "$VM_DIR" ]]; then
        chmod -R u+w "$VM_DIR"
        mv "$VM_DIR" "$old"
    fi
    mv "$dir" "$VM_DIR"
    BUILD_DIR=""
    rm -rf "$old"
}

cmd_build() {
    require_tools
    take_lock 60
    machine_running "$RUN_DIR" && die "the VM is running; stop it first (vm stop)"
    remove_stale_builds
    fetch_verified "$ISO_URL" "$ISO_FILE" "$ISO_SHA256"
    fetch_verified "$OPENSSH_URL" "$OPENSSH_FILE" "$OPENSSH_SHA256"
    BUILD_DIR="$(mktemp -d "$STATE_DIR/build.XXXXXX")"
    trap abandon_build EXIT
    no_cow "$BUILD_DIR"
    make_secrets "$BUILD_DIR"
    make_stick "$BUILD_DIR"
    make_disks "$BUILD_DIR"
    log "installing Windows (about 10 minutes; deadline 2 hours)"
    start_setup "$BUILD_DIR"
    HOSTKEY=accept-new wait_ssh "$BUILD_DIR" "$BUILD_DIR" 7200
    wait_runners "$BUILD_DIR" 300
    provision "$BUILD_DIR" activate 600
    provision "$BUILD_DIR" build-tools 5400
    provision "$BUILD_DIR" rust 1800
    install_opengl_pack "$BUILD_DIR"
    update_windows "$BUILD_DIR"
    seal_base "$BUILD_DIR"
    swap_in "$BUILD_DIR"
    trap - EXIT
    log "built: $(<"$VM_DIR/info.json")"
}
