#!/bin/bash
# Usage: run-unit.sh ARGS...
#
# Runs the spike as the daemon will run: a systemd user service whose unit
# name carries the app id (app-<id>.service), with a hard runtime limit so a
# dropped SSH session cannot leave it running. It runs in the caller's
# directory, so relative paths (a token file) mean the same as outside it.
set -u
here=$(cd "$(dirname "$0")" && pwd)
exec systemd-run --user --quiet --pipe --wait --collect --same-dir \
	--unit=app-io.github.tobylo.AutoClickerX02 -p RuntimeMaxSec=300 \
	"$here/target/release/portal-backend-spike" "$@"
