#!/usr/bin/env bash
# Checks docs/facts.json, the facts the project page shows, against this
# repository: its shape (scripts/facts-shape.jq), each available platform's
# commands against the code blocks under its README heading, the hotkeys
# against the README's table, and that every path it names is a file here and
# every figure's source holds its quote. Prints both texts on a difference.
#
# Usage: scripts/check-facts.sh [ROOT]   (ROOT defaults to this repository)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$here/..}"
readme="$root/README.md"

# Whether $1, a relative path, is a regular file under the root with no
# symbolic link anywhere along it.
file_in_repository() {
    local prefix="$root" part parts
    IFS='/' read -ra parts <<<"$1"
    for part in "${parts[@]}"; do
        prefix="$prefix/$part"
        [[ -L "$prefix" ]] && return 1
    done
    [[ -f "$prefix" ]]
}

# jq parses the file once, and every check reads what it parsed, printed
# again as compact JSON: the document the shape check passed. jq refuses
# bytes that are not JSON, and never prints a NUL.
if ! file_in_repository docs/facts.json; then
    echo "docs/facts.json is not a file in the repository" >&2
    exit 1
fi
if ! facts_json="$(jq -c . "$root/docs/facts.json")"; then
    echo "docs/facts.json is not valid JSON" >&2
    exit 1
fi

# jq on the parsed facts, without the CR a native Windows jq ends each line
# with. Fails, saying so, when jq does.
facts_query() {
    local out
    if ! out="$(jq "$@" <<<"$facts_json")"; then
        echo "jq failed reading docs/facts.json" >&2
        return 1
    fi
    printf '%s\n' "${out//$'\r'/}"
}

shape() {
    local problems
    if ! problems="$(facts_query -r -f "$here/facts-shape.jq")"; then
        echo "the shape check could not run" >&2
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
# that text. A block opens with ``` or ~~~ (three or more) and closes with at
# least as many of the same character; lines inside one are never headings.
readme_commands() {
    awk -v want="$1" '
        { sub(/\r$/, "") }
        !fence && match($0, /^[ \t]*(```+|~~~+)/) {
            fence = substr($0, RSTART, RLENGTH)
            sub(/^[ \t]+/, "", fence)
            next
        }
        fence {
            line = $0
            sub(/^[ \t]+/, "", line)
            sub(/[ \t]+$/, "", line)
            mark = substr(fence, 1, 1)
            if (index(line, fence) == 1 && line ~ ("^[" mark "]+$")) { fence = ""; next }
            if (in_section && $0 != "") print
            next
        }
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
# Fails unless exactly one table has that header.
readme_hotkeys() {
    awk '
        { sub(/\r$/, "") }
        /^\|[ \t]*Hotkey[ \t]*\|[ \t]*Does[ \t]*\|/ { tables++; table = 1; next }
        table && /^\|[ \t]*-/ { next }
        table && /^\|/ {
            split($0, cells, "|")
            cell = cells[2]
            gsub(/^[ \t]+|[ \t]+$/, "", cell)
            print cell
            next
        }
        { table = 0 }
        END { if (tables != 1) exit 3 }
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
    local rows id heading from_readme from_file failed=0
    rows="$(facts_query -r '.platforms[] | select(.status == "available") | "\(.id)\t\(.readme)"')" || return 1
    [[ -n "$rows" ]] || return 0
    while IFS=$'\t' read -r id heading; do
        if ! from_readme="$(readme_commands "$heading")"; then
            echo "README.md has no single heading \"$heading\" (platform $id)" >&2
            failed=1
            continue
        fi
        from_file="$(facts_query -r --arg id "$id" \
            '.platforms[] | select(.id == $id) | .lines[] | .command // empty')" || return 1
        same "README.md's commands under \"$heading\"" "$from_readme" \
            "docs/facts.json's commands for $id" "$from_file" || failed=1
    done <<<"$rows"
    return "$failed"
}

hotkeys_match() {
    local from_readme from_file
    if ! from_readme="$(readme_hotkeys)"; then
        echo "README.md has no single table headed \"| Hotkey | Does |\"" >&2
        return 1
    fi
    from_file="$(facts_query -r '.hotkeys.modifiers as $m | .hotkeys.bindings[] | "\($m)+\(.key)"')" || return 1
    same "README.md's hotkey table" "$from_readme" "docs/facts.json's hotkeys" "$from_file"
}

# Every path the file names is a regular file in the repository.
paths_are_files() {
    local paths path failed=0
    paths="$(facts_query -r '.links[], (.platforms[].note.link // empty), .figures[].source.path')" || return 1
    while IFS= read -r path; do
        if ! file_in_repository "$path"; then
            echo "docs/facts.json names $path, which is not a file in the repository" >&2
            failed=1
        fi
    done <<<"$paths"
    return "$failed"
}

quotes_found() {
    local rows id path quote failed=0
    rows="$(facts_query -r '.figures[] | "\(.id)\t\(.source.path)\t\(.source.quote)"')" || return 1
    while IFS=$'\t' read -r id path quote; do
        if ! grep -qF -- "$quote" "$root/$path"; then
            echo "figure $id: $path does not contain \"$quote\"" >&2
            failed=1
        fi
    done <<<"$rows"
    return "$failed"
}

# The checks below read only fields the shape check vouched for, and the
# quotes only from paths that are files.
shape
failed=0
commands_match || failed=1
hotkeys_match || failed=1
if paths_are_files; then
    quotes_found || failed=1
else
    failed=1
fi
if [[ "$failed" == 1 ]]; then
    exit 1
fi
echo "docs/facts.json matches README.md and its sources"
