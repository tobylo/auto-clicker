#!/usr/bin/env bash
# Refreshes the cached crates.io index entry of every crate in Cargo.lock,
# which cargo-deny reads yanks from and a locked build never refreshes.
# Unlocking one registry crate in a dry run makes cargo re-check every entry
# the locked graph reaches; it writes nothing. scripts/gates.sh runs it before
# the deny gate, the weekly advisory workflow before its check.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Cargo.lock lists each package's name, then version, then source.
spec="$(awk -F'"' '/^name = /{n=$2} /^version = /{v=$2}
    /^source = "registry\+/{print n "@" v; exit}' Cargo.lock)"
if [[ -z "$spec" ]]; then
    exit 0
fi
GIT_TERMINAL_PROMPT=0 timeout --kill-after=10 300 \
    cargo --config net.offline=false update --dry-run --quiet -p "$spec"
