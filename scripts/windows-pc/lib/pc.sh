# The PC's own commands: setup, status, screen, selftest and uninstall, with
# its config. run, ssh and push are the VM's (scripts/windows-vm/lib/remote.sh).
# Sourced by scripts/windows-pc/pc, after base.sh and remote.sh.
# shellcheck shell=bash

PC_GUEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../guest" && pwd)"
SSH_PORT=22
# The PC's host key is recorded under this name, so a new address on the LAN
# needs only the config changed.
SSH_HOST_ALIAS=win11-test
KEYS_DIR="$STATE_DIR"
# Set by read_config.
SSH_HOST=""
SSH_KEY_FILE=""
# Before every command sent: scripts may run (the PC's policy is Windows'
# default, Restricted, and the setup leaves it so), output is UTF-8, and PATH
# holds what tester's logon would, cargo included, as an SSH session's does not.
# shellcheck disable=SC2016 # PowerShell expands these on the PC.
REMOTE_PREFIX='Set-ExecutionPolicy -Scope Process Bypass -Force; [Console]::OutputEncoding = New-Object Text.UTF8Encoding $false; $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User"); '
# Before a job is handed to the runner: C:\wn01 must be the setup's, a plain
# folder the administrators or SYSTEM own, inheriting nothing, that nobody
# else can write (0x1200A9 is read and execute), holding run-job.ps1, or it
# is not run
# (exit 125, as when the runner cannot start a job). Another account could
# have made the folder before the setup ran. Anything that cannot be read
# counts as not the setup's.
# shellcheck disable=SC2016 # PowerShell expands these on the PC.
RUN_JOB_PREFIX='$ours = $false; try { $r = Get-Item -LiteralPath C:\wn01 -Force -ErrorAction Stop; $a = Get-Acl -LiteralPath C:\wn01 -ErrorAction Stop; $t = "S-1-5-32-544", "S-1-5-18"; $w = @($a.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]) | Where-Object { $_.IdentityReference.Value -notin $t -and ([int] $_.FileSystemRights -bor 0x1200A9) -ne 0x1200A9 }); $ours = $r.PSIsContainer -and -not ($r.Attributes -band [IO.FileAttributes]::ReparsePoint) -and $a.GetOwner([Security.Principal.SecurityIdentifier]).Value -in $t -and $a.AreAccessRulesProtected -and $w.Count -eq 0 -and (Test-Path -LiteralPath C:\wn01\run-job.ps1 -PathType Leaf) } catch { }; if (-not $ours) { [Console]::Error.WriteLine("C:\wn01 is not the setup'"'"'s, or has no run-job.ps1: run pc setup"); exit 125 }; '

# Where setup and uninstall copy their scripts on the PC: tester's own temp
# folder, which no other account can write.
STAGE_PARENT='C:\Users\tester\AppData\Local\Temp'
STAGE="$STAGE_PARENT\\wn13-setup"
# The runner's files, from the VM's guest folder, and the PC's own.
RUNNER_FILES=(runner.ps1 JobRunner.cs run-job.ps1 click-test.ps1)
PC_RUNNER_FILES=(screen.ps1 focus.ps1)

# Reads $STATE_DIR/config once into SSH_HOST and SSH_KEY_FILE. It holds
# `host=ADDRESS` and `key=/absolute/path/to/private-key` lines, blank lines
# and # comments, and nothing else; it is parsed, never sourced.
read_config() {
    local file="$STATE_DIR/config" line name value
    SSH_HOST="" SSH_KEY_FILE=""
    [[ -f "$file" ]] || die "no config: write $file with host=ADDRESS and key=/path/to/private-key lines (docs/agents/windows-pc.md)"
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ -z "$line" || "$line" == \#* ]] && continue
        [[ "$line" =~ ^(host|key)=(.+)$ ]] || die "$file: not a host= or key= line: $line"
        name="${BASH_REMATCH[1]}" value="${BASH_REMATCH[2]}"
        case "$name" in
            host)
                [[ -z "$SSH_HOST" ]] || die "$file: host is given twice"
                # A name or an IPv4 address; never one ssh could read as an option.
                [[ "$value" =~ ^[A-Za-z0-9][A-Za-z0-9.-]{0,252}$ ]] || die "$file: host must be a host name or an IPv4 address: $value"
                SSH_HOST="$value"
                ;;
            key)
                [[ -z "$SSH_KEY_FILE" ]] || die "$file: key is given twice"
                [[ "$value" =~ ^/[A-Za-z0-9._/+-]+$ ]] || die "$file: key must be an absolute path of letters, digits and ._/+-: $value"
                SSH_KEY_FILE="$value"
                ;;
        esac
    done <"$file"
    [[ -n "$SSH_HOST" ]] || die "$file: no host= line"
    [[ -n "$SSH_KEY_FILE" ]] || die "$file: no key= line"
    [[ -r "$SSH_KEY_FILE" ]] || die "$file: cannot read the key $SSH_KEY_FILE"
    [[ -s "$KEYS_DIR/known_hosts" ]] ||
        die "no $KEYS_DIR/known_hosts: record the PC's host key under the name $SSH_HOST_ALIAS (docs/agents/windows-pc.md)"
}

# The PC's key must match the recorded one, whatever HOSTKEY the caller's
# environment holds (only a VM build accepts a new key).
HOSTKEY=yes

# The probe reads nothing: stdin belongs to the command that follows
# (`pc ssh` and `pc push` stream files through it).
require_running() {
    vm_ssh 20 "$KEYS_DIR" 'exit 0' </dev/null >/dev/null 2>&1 ||
        die "the PC does not answer SSH at $SSH_HOST (off, asleep, at another address, or a changed host key)"
}

# Runs PowerShell script $2 on the PC within $1 seconds without copying it
# there: it travels inside the command, base64-encoded.
remote_script() {
    local encoded
    encoded="$(base64 -w0 <"$2")"
    vm_ssh "$1" "$KEYS_DIR" "${REMOTE_PREFIX}& ([ScriptBlock]::Create([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$encoded'))))"
}

# Copies files $3... into an empty staging folder on the PC and runs script
# $2 from it within $1 seconds, its stdin this function's; fails when the
# script does, or throws. The script removes the folder when it ends. Each
# step is checked by hand: callers run this before ||, where errexit is off.
run_staged() {
    local seconds="$1" script="$2" local_dir
    shift 2
    local_dir="$(mktemp -d)" || die "could not make a local staging folder"
    # shellcheck disable=SC2064 # The path is fixed now; the trap runs later.
    trap "rm -rf -- '$local_dir'" EXIT
    mkdir "$local_dir/wn13-setup" && cp -- "$@" "$local_dir/wn13-setup/" || die "could not stage the setup's files"
    vm_ssh 60 "$KEYS_DIR" "Remove-Item -LiteralPath '$STAGE' -Recurse -Force -ErrorAction SilentlyContinue; if (Test-Path -LiteralPath '$STAGE') { exit 1 }" </dev/null ||
        die "could not empty $STAGE on the PC"
    vm_push 120 "$KEYS_DIR" "$STAGE_PARENT" "$local_dir/wn13-setup" </dev/null || die "could not copy the setup's files to the PC"
    rm -rf -- "$local_dir"
    trap - EXIT
    vm_ssh "$seconds" "$KEYS_DIR" "${REMOTE_PREFIX}try { & '$STAGE\\$script'; exit \$LASTEXITCODE } catch { [Console]::Error.WriteLine(\$_); exit 1 }"
}

# Makes tester's password unless the dev box has one, and checks it: 24
# letters and digits, one line, readable by the owner alone. It is written
# in full before the PC is touched, so a failed setup never loses it.
ensure_password() {
    local file="$STATE_DIR/password" tmp text
    if [[ ! -e "$file" ]]; then
        tmp="$(mktemp "$STATE_DIR/password.XXXXXX")"
        python3 -I -c '
import secrets, string
alphabet = string.ascii_letters + string.digits
while True:
    password = "".join(secrets.choice(alphabet) for _ in range(24))
    if any(c.islower() for c in password) and any(c.isupper() for c in password) and any(c.isdigit() for c in password):
        print(password)
        break
' >"$tmp"
        sync -- "$tmp"
        mv -- "$tmp" "$file"
        sync -- "$STATE_DIR"
        log "made a new password for tester in $file"
    fi
    chmod 600 "$file"
    # The file is exactly that line and its newline: the PC reads the first
    # line, so nothing may come before or after it. Its bytes are checked
    # first, since bash drops a NUL from what it reads.
    [[ "$(LC_ALL=C tr -d 'A-Za-z0-9\n' <"$file" | wc -c)" == 0 ]] && text="$(<"$file")" &&
        [[ "$text" =~ ^[A-Za-z0-9]{16,128}$ ]] && (($(wc -c <"$file") == ${#text} + 1)) ||
        die "$file must hold one line of 16 to 128 letters and digits"
}

# Whether tester is logged on to the PC's desktop; dies when that cannot be
# read, rather than taking it for nobody.
tester_logged_on() {
    local user
    user="$(vm_ssh 30 "$KEYS_DIR" '(Get-CimInstance Win32_ComputerSystem).UserName' </dev/null | tr -d '\r')" ||
        die "could not read who is logged on to the PC"
    [[ "${user,,}" == *'\tester' ]]
}

cmd_setup() {
    local files=() name
    for name in "${RUNNER_FILES[@]}"; do files+=("$RUNNER_GUEST_DIR/$name"); done
    for name in "${PC_RUNNER_FILES[@]}" setup.ps1 Autologon.cs; do files+=("$PC_GUEST_DIR/$name"); done
    take_lock 600
    ensure_password
    require_running
    log "setting the PC up (deadline 3600 s)"
    run_staged 3600 setup.ps1 "${files[@]}" <"$STATE_DIR/password" || die "the setup failed"
    if tester_logged_on; then
        wait_runners "$KEYS_DIR" 120
        log "both runners run"
    else
        log "tester is not logged on: the runners start at its next logon"
    fi
}

cmd_uninstall() {
    take_lock 600
    require_running
    run_staged 600 uninstall.ps1 "$PC_GUEST_DIR/uninstall.ps1" "$PC_GUEST_DIR/Autologon.cs" </dev/null ||
        die "the uninstall did not finish; run it again"
    log "kept $STATE_DIR/password: tester's password on the PC is still that one"
}

# What has focus in the desktop, from the elevated runner; fails only when
# focus.ps1 says (exit 2) that an elevated window is in front, since Windows
# then drops injected input. A check that did not run is reported and does
# not fail: the click test then fails on its own.
check_focus() {
    local focus rc=0
    focus="$(run_job "$KEYS_DIR" high 30 'C:\wn01' 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\wn01\focus.ps1' </dev/null | tr -d '\r')" || rc=$?
    echo "focus:      ${focus:-unknown (exit $rc)}"
    ((rc != 2))
}

# The runners answer a job each, and what has focus. Only while no other
# session holds the lock: a job of its would hold up these.
status_jobs() {
    local rc=0
    if ! runners_ready "$KEYS_DIR" </dev/null; then
        echo "jobs:       not tried, the runners do not run"
        return 0
    fi
    if ! try_lock 5; then
        echo "jobs:       not tried, another session is using the PC"
        return 0
    fi
    run_job "$KEYS_DIR" normal 30 'C:\wn01' 'exit 0' >/dev/null 2>&1 </dev/null || rc=$?
    if ((rc == 0)); then echo "jobs:       normal answers"; else echo "jobs:       normal: no answer (exit $rc)"; fi
    check_focus || true
}

cmd_status() {
    echo "pc:         tester@$SSH_HOST (config $STATE_DIR/config)"
    if ! vm_ssh 20 "$KEYS_DIR" 'exit 0' >/dev/null 2>&1 </dev/null; then
        echo "state:      no answer over SSH (off, asleep, at another address, or a changed host key)"
        return 1
    fi
    remote_script 60 "$PC_GUEST_DIR/status.ps1" </dev/null | tr -d '\r' || echo "state:      could not be read"
    status_jobs
}

# Captures the desktop through the elevated runner, so a capture can be taken
# while a test runs in the normal queue. Takes no lock, as it changes nothing.
cmd_screen() {
    local out="${1:-wn13-screen.png}" shot
    require_running
    shot="$(mktemp "$STATE_DIR/screen.XXXXXX")"
    if run_job "$KEYS_DIR" high 60 'C:\wn01' 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\wn01\screen.ps1' </dev/null |
        tr -d '\r\n' | base64 -d >"$shot" &&
        [[ "$(head -c 8 "$shot" | od -An -tx1 | tr -d ' \n')" == 89504e470d0a1a0a ]] &&
        mv -fT -- "$shot" "$out"; then
        echo "$out"
        return 0
    fi
    rm -f "$shot"
    die "no screenshot was written to $out"
}

cmd_selftest() {
    take_lock 120
    require_running
    check_focus || die "Windows would drop the injected click: close the elevated window named above"
    check_click
    check_deadline
    cmd_screen "$STATE_DIR/selftest-screen.png" >/dev/null
    log "ok   screen captured: $STATE_DIR/selftest-screen.png"
    log "selftest passed"
}
