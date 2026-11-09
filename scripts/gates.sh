#!/usr/bin/env bash
# Runs the gates every commit must pass, in order, and stops at the first
# failure. CI runs this same script.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if ! command -v cargo >/dev/null 2>&1 && [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi

gate() {
    local name="$1"
    shift
    echo "=== GATE: $name"
    if ! "$@"; then
        echo "=== GATE FAILED: $name" >&2
        exit 1
    fi
}

gate fmt cargo fmt --all --check
gate clippy cargo clippy --workspace --all-targets --locked -- -D warnings
gate test cargo test --workspace --locked

echo "=== ALL GATES PASSED"
