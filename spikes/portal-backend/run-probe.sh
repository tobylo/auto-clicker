#!/bin/bash
# Usage: run-probe.sh PREFIX COMMAND...
#
# Opens the device test's probe window fullscreen through probe_logged.py,
# runs COMMAND once the window is active, then closes the window. Writes
# PREFIX.window (the window's own lines), PREFIX.wayland (the raw Wayland
# events it received, the ground truth for presses and releases; key codes
# removed) and PREFIX.spike (COMMAND's output). Fails when COMMAND fails or a
# log could not be written. Run it under the spike lock.
set -u
prefix=$1
shift
here=$(cd "$(dirname "$0")" && pwd)
python3 "$here/probe_logged.py" "$prefix" &
window=$!

# Closes the window; probe_logged.py exits once both logs are written.
finish() {
	kill -TERM "$window" 2>/dev/null
	wait "$window"
}

for _ in $(seq 100); do
	grep -q '^ready' "$prefix.window" 2>/dev/null && break
	sleep 0.1
done
if ! grep -q '^ready' "$prefix.window" 2>/dev/null; then
	echo "the probe window never became active" >&2
	finish
	exit 1
fi
"$@" 2>&1 | tee "$prefix.spike"
status=${PIPESTATUS[0]}
sleep 1
finish || exit 1
exit "$status"
