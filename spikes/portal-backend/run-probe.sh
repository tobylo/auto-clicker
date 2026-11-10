#!/bin/bash
# Usage: run-probe.sh PREFIX COMMAND...
#
# Opens the device test's probe window fullscreen through probe_logged.py,
# runs COMMAND once the window is active, then closes the window. Writes
# PREFIX.window (the window's own lines), PREFIX.wayland (the raw Wayland
# events it received, the ground truth for presses and releases; key codes
# removed) and PREFIX.spike (COMMAND's output). Fails when COMMAND fails or a
# log could not be written. However the script ends (Ctrl+C and SIGTERM
# included), it closes the window. Run it under the spike lock.
set -u
prefix=$1
shift
here=$(cd "$(dirname "$0")" && pwd)
# A `ready` left by an earlier run must not let input start early.
rm -f "$prefix.window"
python3 "$here/probe_logged.py" "$prefix" &
window=$!

# Closes the window once; probe_logged.py exits after writing both logs.
finished=0
finish() {
	[ "$finished" = 1 ] && return 0
	finished=1
	kill -TERM "$window" 2>/dev/null
	wait "$window"
}
trap finish EXIT
trap 'exit 130' INT TERM

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
statuses=("${PIPESTATUS[@]}")
sleep 1
finish || exit 1
[ "${statuses[1]}" = 0 ] || exit 1
exit "${statuses[0]}"
