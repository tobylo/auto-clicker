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

# The deadlines below need GNU coreutils' `timeout`: on Windows, Git for
# Windows' bash provides it, and Windows' own timeout.exe only waits.
if ! timeout --version 2>/dev/null | grep -q coreutils; then
    echo "GNU timeout (coreutils) is required; found: $(command -v timeout || echo none)" >&2
    exit 1
fi

# No step may wait on a credential prompt.
export GIT_TERMINAL_PROMPT=0

# Runs a command that may wait on the network, killing it after $1 seconds.
bounded() {
    local seconds="$1"
    shift
    timeout --kill-after=10 "$seconds" "$@"
}

# Whether a run may fetch the advisory database: always in CI, and on a
# developer machine when the database's repository answers within 15 seconds.
advisory_db_reachable() {
    [[ -n "${CI:-}" ]] ||
        bounded 15 git ls-remote --exit-code "$ADVISORY_DB_URL" HEAD >/dev/null 2>&1
}

# A Cargo config in the tree could replace crates.io with another registry,
# which cargo-deny would still report as crates.io, or alias a cargo command
# the gates run. The repository tracks no `.cargo` path at all, in any case,
# since Windows reads `.CARGO` as `.cargo`, so this runs before the first
# cargo command.
no_cargo_config() {
    local found
    found="$(git ls-files -- ':(glob,icase)**/.cargo' ':(glob,icase)**/.cargo/**')" || return 1
    if [[ -n "$found" ]]; then
        echo "Cargo configuration is not allowed in the repository: $found" >&2
        return 1
    fi
}

# Checks that every dependency comes from crates.io, has no known advisory, is
# not yanked and carries an allowed licence (deny.toml), with the cargo-deny
# release CI installs. Online, it first refreshes the cached crates.io index
# entries that cargo-deny reads yanks from (scripts/refresh-index.sh); offline, it checks against the cached advisory database, which
# fails once older than deny.toml allows. It builds nothing, so it runs before
# any build script or proc macro does.
deny_check() {
    local want="cargo-deny $CARGO_DENY_VERSION" found
    found="$(cargo deny --version 2>/dev/null || true)"
    if [[ "$found" != "$want" ]]; then
        echo "$want is required (found: ${found:-none}); install it with" \
            "cargo install --locked cargo-deny@$CARGO_DENY_VERSION" >&2
        return 1
    fi
    if advisory_db_reachable; then
        scripts/refresh-index.sh &&
            bounded 900 cargo deny --locked check --deny index-failure
        return
    fi
    echo "$ADVISORY_DB_URL is unreachable: checking against the cached advisory database" >&2
    if ! bounded 300 cargo deny --locked --offline check --deny index-failure; then
        echo "This run was offline. If the error above is about the advisory" \
            "database (stale or missing), connect and run the gates again." >&2
        return 1
    fi
}

# The project page's facts (docs/facts.json) against the README and the
# records they cite, and the cases that check the check.
gate facts scripts/check-facts.sh
gate facts-cases scripts/check-facts-test.sh
gate cargo-config no_cargo_config
gate fmt cargo fmt --all --check
gate deny deny_check
gate clippy cargo clippy --workspace --all-targets --locked -- -D warnings
# CI on Windows runs this gate in a step of its own, outside Git for Windows'
# bash, and sets GATES_TEST_APART=1 (ci.yml says why).
if [[ "${GATES_TEST_APART:-}" == 1 ]]; then
    echo "=== ALL GATES PASSED BUT test, which the caller runs"
    exit 0
fi
gate test cargo test --workspace --locked

echo "=== ALL GATES PASSED"
