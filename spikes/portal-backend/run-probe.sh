#!/bin/bash
# Usage: run-probe.sh PREFIX COMMAND...
#
# Opens the device test's probe window fullscreen with WAYLAND_DEBUG=1, runs
# COMMAND once the window is active, then closes the window. Writes
# PREFIX.window (the window's own lines), PREFIX.wayland (the raw Wayland
# events it received, the ground truth for presses and releases) and
# PREFIX.spike (COMMAND's output). Run it under the spike lock.
set -u
prefix=$1
shift
WAYLAND_DEBUG=1 python3 probe_window.py >"$prefix.window" 2>"$prefix.wayland" &
window=$!
trap 'kill "$window" 2>/dev/null; wait "$window" 2>/dev/null' EXIT
for _ in $(seq 100); do
	grep -q '^ready' "$prefix.window" && break
	sleep 0.1
done
if ! grep -q '^ready' "$prefix.window"; then
	echo "the probe window never became active" >&2
	exit 1
fi
"$@" 2>&1 | tee "$prefix.spike"
status=${PIPESTATUS[0]}
sleep 1
exit "$status"
