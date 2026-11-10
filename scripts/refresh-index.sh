#!/usr/bin/env bash
# Refreshes the cached crates.io index entry of every crate in Cargo.lock,
# which cargo-deny reads yanks from and a locked build never refreshes.
# Unlocking one registry crate in a dry run makes cargo re-check every entry
# the locked graph reaches; it writes nothing. scripts/gates.sh runs it before
# the deny gate, the weekly advisory workflow before its check.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Cargo resolves the locked graph itself, whatever Cargo.lock's formatting;
# no registry package in it means there is nothing to refresh.
export GIT_TERMINAL_PROMPT=0
spec="$(timeout --kill-after=10 300 cargo metadata --locked --format-version 1 | jq -r '
    [.packages[] | select((.source // "") | startswith("registry+"))
        | "\(.name)@\(.version)"] | first // empty')"
if [[ -z "$spec" ]]; then
    exit 0
fi
timeout --kill-after=10 300 cargo --config net.offline=false \
    update --dry-run --quiet -p "$spec"
