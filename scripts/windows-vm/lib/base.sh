# Helpers shared by scripts/windows-vm/vm and scripts/windows-pc/pc: logging,
# deadlines, arguments and the lock. Sourced, not run. The caller sets
# TOOL_NAME (the command's name in messages) and MACHINE_NAME first.
# shellcheck shell=bash

# The script's own stderr, kept as fd 8: an exit trap that runs while a
# command's stderr is sent elsewhere (`vm_ssh ... 2>/dev/null`) still reports.
exec 8>&2

log() {
    printf '%s %s: %s\n' "$(date -u +%H:%M:%S)" "$TOOL_NAME" "$*" >&8
}

die() {
    log "error: $*"
    exit 1
}

# Sets STATE_DIR to $1 with each symbolic link resolved, and LOCK_FILE in it.
# Only an absolute path of letters, digits and ._/+- is taken.
set_state_dir() {
    STATE_DIR="$1"
    [[ "$STATE_DIR" == /* ]] && STATE_DIR="$(realpath -m -- "$STATE_DIR")" &&
        [[ "$STATE_DIR" =~ ^/[A-Za-z0-9._/+-]+$ ]] || {
        echo "$TOOL_NAME: the state directory must be an absolute path whose resolved form has letters, digits and ._/+- only: '$STATE_DIR'" >&2
        exit 2
    }
    LOCK_FILE="$STATE_DIR/lock"
}

# Runs a command, killing it after $1 seconds. --foreground keeps it in our
# process group, so Ctrl-C reaches it; 9>&- keeps it from holding the
# machine's lock should it outlive us, and 8>&- from holding our stderr open.
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

# Opens the lock file as fd 9 and waits at most $1 seconds for the lock;
# fails, holding nothing, when another session keeps it.
try_lock() {
    # Checked by hand: callers run this before ||, where errexit is off.
    mkdir -p "$STATE_DIR" && chmod 700 "$STATE_DIR" || die "could not make $STATE_DIR private"
    exec 9>"$LOCK_FILE" || die "could not open $LOCK_FILE"
    flock -w "$1" 9 || {
        exec 9>&-
        return 1
    }
}

# Takes the machine's lock for the rest of this process, waiting at most $1
# seconds: one session at a time changes the machine or runs a job on it.
# Daemons started under it close fd 9, or they would hold it after we exit.
take_lock() {
    try_lock "$1" || die "another session is using $MACHINE_NAME (lock held for over $1 s)"
}
