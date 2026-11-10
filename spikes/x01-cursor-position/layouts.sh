#!/bin/bash
# X01 mapping check across display layouts. Holds the shared spike lock for
# the whole run and restores the owner's layout on every exit:
# eDP-1 at 0,0 scale 1.25; DP-1 at 2048,0 scale 1, mode 1. A failed layout
# change or measurement ends the run with its status.
set -u
cd "$(dirname "$0")"
restore() {
    kscreen-doctor output.DP-1.enable output.DP-1.mode.1 output.DP-1.scale.1 \
        output.eDP-1.scale.1.25 output.eDP-1.position.0,0 output.DP-1.position.2048,0 >/dev/null
}
trap 'restore || echo "WARNING: could not restore the display layout" >&2' EXIT
check() {
    local title=$1 monitors=$2
    shift 2
    echo "=== $title"
    restore || exit 1
    if [ $# -gt 0 ]; then kscreen-doctor "$@" >/dev/null || exit 1; fi
    sleep 3
    kscreen-doctor -o | sed 's/\x1b\[[0-9;]*m//g' | grep -E "^Output|Geometry|Scale"
    local out status
    out=$(python3 harness/x01.py mapping --monitors "$monitors" 2>&1)
    status=$?
    grep -v -E "Gtk-WARNING|^$" <<<"$out"
    [ "$status" -eq 0 ] || { echo "mapping failed with status $status"; exit "$status"; }
}
check "dual, DP-1 right at scale 1" 2
check "panel alone" 1 output.DP-1.disable
check "dual, DP-1 left" 2 output.DP-1.position.0,0 output.eDP-1.position.3440,0
check "dual, DP-1 above" 2 output.DP-1.position.0,0 output.eDP-1.position.0,1440
check "dual, DP-1 right at scale 1.5" 2 output.DP-1.scale.1.5
check "dual, DP-1 at negative x" 2 output.DP-1.position.-3440,0
