#!/usr/bin/env bash
# Runs scripts/check-facts.sh against changed copies of README.md,
# docs/facts.json and the files it names, and checks that each change passes
# or fails as it should, with the texts a reader needs.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/.."

# The cases change copies of a tree that passes, and copy only paths the gate
# has checked are files in this repository.
if ! "$here/check-facts.sh" >/dev/null; then
    echo "scripts/check-facts.sh fails on this repository; fix that first" >&2
    exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A fresh copy of every file the gate reads, in $work/tree.
fresh_tree() {
    local tree="$work/tree" path
    rm -rf "$tree"
    mkdir -p "$tree/docs"
    cp "$repo/README.md" "$tree/README.md"
    cp "$repo/docs/facts.json" "$tree/docs/facts.json"
    while IFS= read -r path; do
        mkdir -p "$tree/$(dirname "$path")"
        cp "$repo/$path" "$tree/$path"
    done < <(jq -r '.links[], (.platforms[].note.link // empty), .figures[].source.path' \
        "$repo/docs/facts.json" | tr -d '\r' | sort -u)
}

# Changes the copy of docs/facts.json with the jq filter $1.
edit_facts() {
    local file="$work/tree/docs/facts.json"
    jq "$1" "$file" >"$work/facts.json" && mv "$work/facts.json" "$file"
}

# Puts a jq first on PATH that fails whenever its filter mentions $1, and
# otherwise runs the real one.
failing_jq() {
    local real
    real="$(command -v jq)"
    mkdir -p "$work/bin"
    printf '#!/usr/bin/env bash\ncase "$*" in *%s*) exit 5 ;; esac\nexec "%s" "$@"\n' "$1" "$real" >"$work/bin/jq"
    chmod +x "$work/bin/jq"
}

# Replaces the copy of docs/guide/install.md with a link to README.md.
link_install_guide() {
    rm "$work/tree/docs/guide/install.md"
    ln -s ../../README.md "$work/tree/docs/guide/install.md"
}

# Changes the copy of README.md with the sed script $1.
edit_readme() {
    sed -i.bak -e "$1" "$work/tree/README.md" && rm "$work/tree/README.md.bak"
}

# Cuts the copy of docs/facts.json off halfway.
truncate_facts() {
    local file="$work/tree/docs/facts.json"
    head -c 200 "$file" >"$work/facts.json" && mv "$work/facts.json" "$file"
}

failures=0

# Runs case $1 after the change in "${@:4}", expecting outcome $2 (pass or
# fail) with every |-separated text in $3 in the output.
case_() {
    local name="$1" want="$2" texts="$3" output got text
    shift 3
    fresh_tree
    "$@"
    if output="$(PATH="$work/bin:$PATH" "$here/check-facts.sh" "$work/tree" 2>&1)"; then got=pass; else got=fail; fi
    rm -rf "$work/bin"
    if [[ "$got" != "$want" ]]; then
        echo "FAIL $name: expected $want, got $got" >&2
        echo "$output" >&2
        failures=$((failures + 1))
        return
    fi
    IFS='|' read -ra wanted <<<"$texts"
    for text in "${wanted[@]}"; do
        if [[ "$output" != *"$text"* ]]; then
            echo "FAIL $name: output lacks \"$text\"" >&2
            echo "$output" >&2
            failures=$((failures + 1))
            return
        fi
    done
    echo "ok   $name"
}

case_ unchanged pass "matches README.md" true
case_ readme_command_changed fail "sudo pacman -Syyu|sudo pacman -Syu|README.md's commands under \"Install\"" \
    edit_readme 's/^sudo pacman -Syu$/sudo pacman -Syyu/'
case_ readme_command_added fail "makepkg --printsrcinfo" \
    edit_readme '/^cd ~\/auto-clicker\/packaging\/arch/a\
makepkg --printsrcinfo'
case_ readme_command_removed fail "docs/facts.json's commands for arch" \
    edit_readme '/^sudo usermod -aG input NAME$/d'
case_ readme_block_added fail "echo stray" \
    edit_readme '/^The icon appears in the tray\.$/a\
\
```fish\
echo stray\
```'
case_ readme_tilde_block_added fail "echo tilde" \
    edit_readme '/^The icon appears in the tray\.$/a\
\
~~~fish\
echo tilde\
~~~'
case_ readme_heading_twice fail "no single heading \"Install\"" \
    edit_readme 's/^## Documentation$/## Install/'
case_ readme_heading_renamed fail "no single heading \"Install\"" \
    edit_readme 's/^## Install$/## Installing/'
case_ file_command_changed fail "systemctl --user enable auto-clicker" \
    edit_facts '(.platforms[0].lines[] | select(.command? == "systemctl --user enable --now auto-clicker")).command = "systemctl --user enable auto-clicker"'
case_ file_remark_added pass "matches README.md" \
    edit_facts '.platforms[0].lines += [{"remark": "that is all"}]'
case_ fish_comment_is_not_a_heading fail "# a comment" \
    edit_readme 's/^sudo pacman -Syu$/# a comment\
sudo pacman -Syu/'
case_ readme_hotkey_changed fail "Ctrl+Alt+F6|Ctrl+Shift+F6|hotkey table" \
    edit_readme 's/^| Ctrl+Shift+F6 |/| Ctrl+Alt+F6 |/'
case_ readme_hotkey_table_twice fail "no single table" \
    edit_readme 's/^## Install$/| Hotkey | Does |\
| --- | --- |\
| Ctrl+Shift+F6 | Old |\
\
## Install/'
case_ file_hotkey_changed fail "Ctrl+Shift+F9" \
    edit_facts '.hotkeys.bindings[2].key = "F9"'
case_ not_json fail "not valid JSON" \
    truncate_facts
case_ unknown_key fail "wrong shape|exactly version, links" \
    edit_facts '.extra = 1'
case_ wrong_status fail "status must be available or soon" \
    edit_facts '.platforms[1].status = "beta"'
case_ soon_with_lines fail "soon has no install lines" \
    edit_facts '.platforms[1].lines = [{"command": "winget install auto-clicker"}]'
case_ available_without_readme fail "names its README heading" \
    edit_facts 'del(.platforms[0].readme)'
case_ guide_only_false fail "guideOnly may only be true" \
    edit_facts '.platforms[0].lines[4].guideOnly = false'
case_ path_outside fail "must be a path in this repository" \
    edit_facts '.links.guide = "../README.md"'
case_ path_missing fail "docs/guide/gone.md, which is not a file in the repository" \
    edit_facts '.links.guide = "docs/guide/gone.md"'
case_ jq_fails_on_commands fail "" failing_jq 'select(.status'
case_ jq_fails_on_command_lines fail "" failing_jq '.command // empty'
case_ jq_fails_on_hotkeys fail "" failing_jq 'hotkeys.modifiers as'
case_ jq_fails_on_paths fail "" failing_jq '.links[]'
case_ jq_fails_on_quotes fail "" failing_jq 'source.quote'
if ln -s README.md "$work/probe" 2>/dev/null && [[ -L "$work/probe" ]]; then
    case_ path_is_a_link fail "docs/guide/install.md, which is not a file" link_install_guide
else
    echo "skip path_is_a_link: this system makes no symbolic links"
fi
case_ quote_missing fail "figure idle-memory: docs/planning/map.md does not contain" \
    edit_facts '.figures[2].source.quote = "idles at 9.9 MiB"'

if [[ "$failures" -gt 0 ]]; then
    echo "$failures check-facts case(s) failed" >&2
    exit 1
fi
echo "all check-facts cases passed"
