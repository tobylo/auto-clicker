#!/bin/bash
# Usage: run-unit.sh ARGS...
#
# Runs the spike as the daemon will run: a systemd user service whose unit
# name carries the app id (app-<id>.service), with a hard runtime limit so a
# dropped SSH session cannot leave it running.
set -u
here=$(cd "$(dirname "$0")" && pwd)
exec systemd-run --user --quiet --pipe --wait --collect \
	--unit=app-io.github.tobylo.AutoClickerX02 -p RuntimeMaxSec=300 \
	"$here/target/release/portal-backend-spike" "$@"
