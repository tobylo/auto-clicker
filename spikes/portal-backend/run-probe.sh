#!/bin/bash
# Usage: run-probe.sh PREFIX COMMAND...
#
# Opens the device test's probe window fullscreen with WAYLAND_DEBUG=1, runs
# COMMAND once the window is active, then closes the window. Writes
# PREFIX.window (the window's own lines), PREFIX.wayland (the raw Wayland
# events it received, the ground truth for presses and releases) and
# PREFIX.spike (COMMAND's output). Key codes are removed from both window
# logs before they reach the disk: whatever is typed while the window has
# focus is never stored. Returns only once both logs are complete. Run it
# under the spike lock.
set -u
prefix=$1
shift

# Open both logs here, so a log that cannot be written stops the run.
exec 3>"$prefix.window" 4>"$prefix.wayland" || exit 1
# `key 30` -> `key`; wl_keyboard.key(serial, time, 30, 1) -> (serial, time, ?, 1)
exec 5> >(sed -u -E 's/^key [0-9]+/key/' >&3)
window_filter=$!
exec 6> >(sed -u -E 's/(wl_keyboard[#@][0-9]+\.key\([0-9]+, [0-9]+, )[0-9]+/\1?/' >&4)
wayland_filter=$!
exec 3>&- 4>&-

WAYLAND_DEBUG=1 python3 probe_window.py >&5 2>&6 &
window=$!

# Stops the window, closes the filters' input, and waits up to 10 s for each
# filter to write its last lines.
finish() {
	kill "$window" 2>/dev/null
	wait "$window" 2>/dev/null
	exec 5>&- 6>&-
	for filter in "$window_filter" "$wayland_filter"; do
		for _ in $(seq 100); do
			kill -0 "$filter" 2>/dev/null || break
			sleep 0.1
		done
		if kill -0 "$filter" 2>/dev/null; then
			echo "a log filter did not finish; the logs are incomplete" >&2
			exit 1
		fi
	done
}
trap finish EXIT

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
