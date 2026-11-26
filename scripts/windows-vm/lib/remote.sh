# Reaching a Windows machine that runs the desktop runner (guest/runner.ps1):
# SSH, copying files in, handing jobs to the runner, and the commands and
# checks built on them. Shared by scripts/windows-vm/vm and
# scripts/windows-pc/pc, after lib/base.sh. The caller sets:
#   SSH_PORT       the port of the machine's sshd
#   SSH_HOST       its address (default 127.0.0.1, the VM's forward)
#   SSH_KEY_FILE   the private key (default: key in the keys directory)
#   SSH_HOST_ALIAS the name its host key is recorded under (default: none)
#   KEYS_DIR       the directory with known_hosts, for the commands below
#   REMOTE_PREFIX  PowerShell put before each command they send (default none)
# and defines require_running.
# shellcheck shell=bash

RUNNER_GUEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../guest" && pwd)"

# SSH to tester on the machine whose known_hosts (and, for the VM, key) are
# in $2, ended after $1 seconds. HOSTKEY=accept-new only while a build first
# meets the VM; afterwards the recorded key must match. (A function, so
# callers pass the deadline here: `bounded` cannot run a shell function.)
vm_ssh() {
    local seconds="$1" keys="$2" alias=()
    shift 2
    [[ -n "${SSH_HOST_ALIAS:-}" ]] && alias=(-o HostKeyAlias="$SSH_HOST_ALIAS")
    bounded "$seconds" ssh -F /dev/null -p "$SSH_PORT" -i "${SSH_KEY_FILE:-$keys/key}" -o IdentitiesOnly=yes \
        -o UserKnownHostsFile="$keys/known_hosts" -o StrictHostKeyChecking="${HOSTKEY:-yes}" "${alias[@]}" \
        -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 \
        -o LogLevel=ERROR tester@"${SSH_HOST:-127.0.0.1}" "$@"
}

# Copies local file or folder $4 into folder $3 on the machine whose keys are
# in $2, within $1 seconds, as a tar stream over SSH: Windows' OpenSSH server
# ends its SFTP subsystem at once, so scp and sftp do not work.
vm_push() {
    local seconds="$1" keys="$2" dest="$3" path="$4"
    [[ "$dest" =~ ^[A-Za-z]:[A-Za-z0-9_.\\-]*$ ]] || die "use a Windows folder without spaces or quotes: $dest"
    [[ -e "$path" ]] || die "no such file or folder: $path"
    tar -c -f - -C "$(dirname "$path")" -- "$(basename "$path")" |
        vm_ssh "$seconds" "$keys" "New-Item -ItemType Directory -Force $dest | Out-Null; cmd /c 'tar -x -f - -C $dest'; exit \$LASTEXITCODE"
}

# Whether both runner tasks run (each writes its pid to `ready` when it
# starts). A `ready` from an earlier boot names a pid some other process may
# have now, so it must be newer than the boot, and the process must be a
# PowerShell that started before it wrote the file. One check, within 60 s.
runners_ready() {
    local check
    check='$boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime; foreach ($q in "normal","high") { $f = Get-Item "C:\wn01\queue\$q\ready" -ErrorAction Stop; if ($f.LastWriteTime -lt $boot) { exit 1 }; $p = Get-Process -Id ([int](Get-Content -LiteralPath $f.FullName -ErrorAction Stop)) -ErrorAction Stop; if ($p.ProcessName -ne "powershell" -or $p.StartTime -gt $f.LastWriteTime) { exit 1 } }'
    vm_ssh 60 "$1" "$check" >/dev/null 2>&1
}

# Waits up to $2 seconds until both runner tasks on the machine with keys in
# $1 run.
wait_runners() {
    local dir="$1" deadline=$((SECONDS + $2))
    until runners_ready "$dir"; do
        ((SECONDS < deadline)) || die "the runner tasks did not start within $2 s"
        sleep 5
    done
}

# Hands a command to a runner queue in the desktop and returns its exit code.
run_job() {
    local keys="$1" queue="$2" seconds="$3" workdir="$4" command="$5" encoded
    encoded="$(printf '%s' "$command" | base64 -w0)"
    vm_ssh $((seconds + 120)) "$keys" \
        "${REMOTE_PREFIX:-}C:\\wn01\\run-job.ps1 -CommandBase64 $encoded -TimeoutSeconds $seconds -Queue $queue -WorkDir '$workdir'; exit \$LASTEXITCODE"
}

# How long `ssh` and `push` may take: WN01_SSH_TIMEOUT, default an hour.
ssh_seconds() {
    seconds_arg WN01_SSH_TIMEOUT "${WN01_SSH_TIMEOUT:-3600}" 86400
}

cmd_ssh() {
    local seconds
    seconds="$(ssh_seconds)"
    take_lock 600
    require_running
    # The prefix goes only before a command: alone, it would turn the
    # interactive shell into a one-off command.
    if (($#)) && [[ -n "${REMOTE_PREFIX:-}" ]]; then
        vm_ssh "$seconds" "$KEYS_DIR" "$REMOTE_PREFIX" "$@"
    else
        vm_ssh "$seconds" "$KEYS_DIR" "$@"
    fi
}

cmd_push() {
    (($# >= 1 && $# <= 2)) || die "usage: $TOOL_NAME push LOCAL_FILE_OR_FOLDER [WINDOWS_FOLDER, default C:\\work]"
    local seconds
    seconds="$(ssh_seconds)"
    take_lock 600
    require_running
    vm_push "$seconds" "$KEYS_DIR" "${2:-C:\\work}" "$1"
}

# The value after option $1, or a usage error when there is none.
option_value() {
    (($# >= 2)) || die "$1 takes a value"
    printf '%s' "$2"
}

cmd_run() {
    local queue=normal seconds=120 workdir='C:\work'
    while (($#)); do
        case "$1" in
            --elevated) queue=high ;;
            --timeout) seconds="$(option_value "$@")"; shift ;;
            --workdir) workdir="$(option_value "$@")"; shift ;;
            --) shift; break ;;
            *) break ;;
        esac
        shift
    done
    (($#)) || die "usage: $TOOL_NAME run [--elevated] [--timeout SECONDS] [--workdir DIR] -- COMMAND..."
    seconds="$(seconds_arg --timeout "$seconds" 3600)"
    # It goes inside PowerShell quotes, which also end at typographic quotes.
    [[ "$workdir" =~ ^[A-Za-z]:[A-Za-z0-9_.\\-]*$ ]] || die "use a Windows folder without spaces or quotes for --workdir: $workdir"
    take_lock "$seconds"
    require_running
    run_job "$KEYS_DIR" "$queue" "$seconds" "$workdir" "$*"
}

check_click() {
    log "running the click test in the desktop"
    vm_push 60 "$KEYS_DIR" "C:\\work" "$RUNNER_GUEST_DIR/click-test.ps1"
    run_job "$KEYS_DIR" normal 60 'C:\work' \
        'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\work\click-test.ps1' ||
        die "the injected click did not reach the test window"
}

# The ids of the machine's ping processes, one per line.
ping_ids() {
    vm_ssh 60 "$KEYS_DIR" '@(Get-Process -Name PING -ErrorAction SilentlyContinue).Id' | tr -d '\r'
}

# A job that outlives its deadline is ended with everything it started: a
# ping in the background and one in front, both far longer than the deadline.
# Pings that ran before the job are not its own and are left out.
check_deadline() {
    local rc=0 before after left
    before="$(ping_ids)" || die "could not list $MACHINE_NAME's processes"
    log "running a job past its 5 s deadline"
    run_job "$KEYS_DIR" normal 5 'C:\work' 'start /b ping -n 600 127.0.0.1 >NUL & ping -n 600 127.0.0.1 >NUL' >/dev/null 2>&1 || rc=$?
    ((rc == 124)) || die "a job past its deadline returned $rc, not 124"
    after="$(ping_ids)" || die "could not list $MACHINE_NAME's processes"
    left="$(grep -cvxF -f <(echo "$before") <<<"$after" || true)"
    [[ "$left" == 0 ]] || die "$left ping process(es) outlived the job's deadline"
    log "ok   a job past its deadline was ended with its whole tree"
}
