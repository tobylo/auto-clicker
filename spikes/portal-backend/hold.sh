#!/bin/bash
# Usage: hold.sh button|key kill|close|eis|stop|exit
#
# One hold case: the spike presses and holds, then ends the way HOW says.
# For HOW=kill this script sends the SIGKILL itself, 5 s after starting (the
# spike holds from about 1 s in and announces the kill at about 3 s). Run it
# inside run-probe.sh, from the directory that holds rd.token.
set -u
what=$1
how=$2
if [ "$how" = kill ]; then
	(
		sleep 5
		systemctl --user kill -s KILL app-io.github.tobylo.AutoClickerX02.service
	) &
	# A run that ends early must not leave the timer to kill a later one.
	timer=$!
	trap 'kill "$timer" 2>/dev/null; wait "$timer" 2>/dev/null' EXIT
fi
"$(dirname "$0")/run-unit.sh" --deadline 30 output "$PWD/rd.token" hold "$what" "$how"
