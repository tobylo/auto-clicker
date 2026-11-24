#!/usr/bin/env bash
# Checks docs/facts.json, the facts the project page shows, against this
# repository: its shape (scripts/facts-shape.jq), each available platform's
# commands against the code blocks under its README heading, the hotkeys
# against the README's table, and that every path it names exists and every
# figure's source holds its quote. Prints both texts on a difference.
#
# Usage: scripts/check-facts.sh [ROOT]   (ROOT defaults to this repository)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$here/..}"
facts="$root/docs/facts.json"
readme="$root/README.md"

# jq, without the CR a native Windows jq ends each line with.
jqr() {
    jq "$@" | tr -d '\r'
}

shape() {
    local problems
    if ! problems="$(jqr -r -f "$here/facts-shape.jq" "$facts")"; then
        echo "docs/facts.json is not valid JSON" >&2
        return 1
    fi
    if [[ -n "$problems" ]]; then
        echo "docs/facts.json has the wrong shape:" >&2
        echo "$problems" >&2
        return 1
    fi
}

# The lines of every code block under the README heading $1 (any level), up to
# the next heading, blank lines left out. Fails unless exactly one heading has
# that text. Lines inside code blocks are never read as headings.
readme_commands() {
    awk -v want="$1" '
        { sub(/\r$/, "") }
        /^[ \t]*```/ { fence = !fence; next }
        fence { if (in_section && $0 != "") print; next }
        /^#+[ \t]/ {
            heading = $0
            sub(/^#+[ \t]+/, "", heading)
            in_section = (heading == want)
            if (in_section) found++
        }
        END { if (found != 1) exit 3 }
    ' "$readme"
}

# The first cell of each row of the README table headed "| Hotkey | Does |".
readme_hotkeys() {
    awk '
        { sub(/\r$/, "") }
        /^\|[ \t]*Hotkey[ \t]*\|[ \t]*Does[ \t]*\|/ { table = 1; next }
        table && /^\|[ \t]*-/ { next }
        table && /^\|/ {
            split($0, cells, "|")
            cell = cells[2]
            gsub(/^[ \t]+|[ \t]+$/, "", cell)
            print cell
            next
        }
        table { exit }
    ' "$readme"
}

# Fails, showing both texts, when $2 (the README's) and $4 (the file's) differ.
same() {
    if [[ "$2" != "$4" ]]; then
        echo "$1 differs from $3." >&2
        printf '%s:\n%s\n\n%s:\n%s\n' "$1" "$2" "$3" "$4" >&2
        return 1
    fi
}

commands_match() {
    local id heading from_readme from_file failed=0
    while IFS=$'\t' read -r id heading; do
        if ! from_readme="$(readme_commands "$heading")"; then
            echo "README.md has no single heading \"$heading\" (platform $id)" >&2
            failed=1
            continue
        fi
        from_file="$(jqr -r --arg id "$id" \
            '.platforms[] | select(.id == $id) | .lines[] | .command // empty' "$facts")"
        same "README.md's commands under \"$heading\"" "$from_readme" \
            "docs/facts.json's commands for $id" "$from_file" || failed=1
    done < <(jqr -r '.platforms[] | select(.status == "available") | "\(.id)\t\(.readme)"' "$facts")
    return "$failed"
}

hotkeys_match() {
    same "README.md's hotkey table" "$(readme_hotkeys)" \
        "docs/facts.json's hotkeys" \
        "$(jqr -r '.hotkeys.modifiers as $m | .hotkeys.bindings[] | "\($m)+\(.key)"' "$facts")"
}

paths_exist() {
    local path failed=0
    while IFS= read -r path; do
        if [[ ! -f "$root/$path" ]]; then
            echo "docs/facts.json names $path, which is not in the repository" >&2
            failed=1
        fi
    done < <(jqr -r '.links[], (.platforms[].note.link // empty), .figures[].source.path' "$facts")
    return "$failed"
}

quotes_found() {
    local id path quote failed=0
    while IFS=$'\t' read -r id path quote; do
        if ! grep -qF -- "$quote" "$root/$path"; then
            echo "figure $id: $path does not contain \"$quote\"" >&2
            failed=1
        fi
    done < <(jqr -r '.figures[] | "\(.id)\t\(.source.path)\t\(.source.quote)"' "$facts")
    return "$failed"
}

# Every check below reads fields the shape check has vouched for.
shape
failed=0
commands_match || failed=1
hotkeys_match || failed=1
paths_exist || failed=1
quotes_found || failed=1
if [[ "$failed" == 1 ]]; then
    exit 1
fi
echo "docs/facts.json matches README.md and its sources"
