#!/usr/bin/env bash
# Runs the gates every commit must pass, in order, and stops at the first
# failure. CI runs this same script.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if ! command -v cargo >/dev/null 2>&1 && [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi

# CI installs this version (.github/workflows/ci.yml); deny_check refuses any other.
CARGO_DENY_VERSION=0.20.2
ADVISORY_DB_URL=https://github.com/rustsec/advisory-db

gate() {
    local name="$1"
    shift
    echo "=== GATE: $name"
    if ! "$@"; then
        echo "=== GATE FAILED: $name" >&2
        exit 1
    fi
}

# Whether a run may fetch the advisory database: always in CI, and on a
# developer machine when the database's repository answers within 15 seconds.
advisory_db_reachable() {
    [[ -n "${CI:-}" ]] || GIT_TERMINAL_PROMPT=0 timeout 15 \
        git ls-remote --exit-code "$ADVISORY_DB_URL" HEAD >/dev/null 2>&1
}

# Checks that every dependency comes from crates.io, has no known advisory and
# carries an allowed licence (deny.toml), with the cargo-deny release CI
# installs. Without network it checks against the cached advisory database,
# which fails once older than deny.toml allows. It reads Cargo.lock and the
# manifests and builds nothing, so it runs before any build script or proc
# macro does.
deny_check() {
    local want="cargo-deny $CARGO_DENY_VERSION" found
    found="$(cargo deny --version 2>/dev/null || true)"
    if [[ "$found" != "$want" ]]; then
        echo "$want is required (found: ${found:-none}); install it with" \
            "cargo install --locked cargo-deny@$CARGO_DENY_VERSION" >&2
        return 1
    fi
    if advisory_db_reachable; then
        cargo deny --locked check
        return
    fi
    echo "$ADVISORY_DB_URL is unreachable: checking against the cached advisory database" >&2
    if ! cargo deny --locked --offline check; then
        echo "This run was offline. If the error above is about the advisory" \
            "database (stale or missing), connect and run the gates again." >&2
        return 1
    fi
}

gate fmt cargo fmt --all --check
gate deny deny_check
gate clippy cargo clippy --workspace --all-targets --locked -- -D warnings
gate test cargo test --workspace --locked

echo "=== ALL GATES PASSED"
