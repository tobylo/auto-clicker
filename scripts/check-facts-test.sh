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

# A jq first on PATH for the gate: it fails when its arguments contain
# $FAIL_JQ_ON, if set, and otherwise runs the real one. The pattern and the
# real jq reach it through the environment, so no text of theirs becomes
# shell code.
REAL_JQ="$(command -v jq)"
export REAL_JQ
mkdir "$work/bin"
cat >"$work/bin/jq" <<'EOF'
#!/usr/bin/env bash
if [[ -n "${FAIL_JQ_ON:-}" && "$*" == *"$FAIL_JQ_ON"* ]]; then exit 5; fi
exec "$REAL_JQ" "$@"
EOF
chmod +x "$work/bin/jq"

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

# Makes the gate's jq fail whenever its arguments contain $1.
failing_jq() {
    export FAIL_JQ_ON="$1"
}

# Points docs/facts.json's install link through a linked directory.
link_guide_directory() {
    ln -s guide "$work/tree/docs/linked"
    edit_facts '.links.install = "docs/linked/install.md"'
}

# Puts a NUL byte inside a key of the copy of docs/facts.json.
put_nul_in_facts() {
    local file="$work/tree/docs/facts.json"
    sed 's/"version"/"ver\x00sion"/' "$file" >"$work/facts.json" && mv "$work/facts.json" "$file"
}

# Makes the copy of docs/facts.json two JSON values: the file, then itself.
two_facts_values() {
    local file="$work/tree/docs/facts.json"
    cat "$file" "$file" >"$work/facts.json" && mv "$work/facts.json" "$file"
}

# Replaces the copy of docs/facts.json with a link to a copy of it.
link_facts() {
    mv "$work/tree/docs/facts.json" "$work/tree/facts-elsewhere.json"
    ln -s ../facts-elsewhere.json "$work/tree/docs/facts.json"
}

# Replaces the copy of README.md with a link to a copy of it.
link_readme() {
    mv "$work/tree/README.md" "$work/tree/README-elsewhere.md"
    ln -s README-elsewhere.md "$work/tree/README.md"
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
# fail) with every |-separated text in $3 in the output, and none of those
# that start with !.
case_() {
    local name="$1" want="$2" texts="$3" output got text
    shift 3
    fresh_tree
    "$@"
    if output="$(PATH="$work/bin:$PATH" "$here/check-facts.sh" "$work/tree" 2>&1)"; then got=pass; else got=fail; fi
    unset FAIL_JQ_ON
    if [[ "$got" != "$want" ]]; then
        echo "FAIL $name: expected $want, got $got" >&2
        echo "$output" >&2
        failures=$((failures + 1))
        return
    fi
    IFS='|' read -ra wanted <<<"$texts"
    for text in "${wanted[@]}"; do
        if [[ "$text" == '!'* ]]; then
            if [[ "$output" == *"${text#!}"* ]]; then
                echo "FAIL $name: output has \"${text#!}\"" >&2
                echo "$output" >&2
                failures=$((failures + 1))
                return
            fi
        elif [[ "$output" != *"$text"* ]]; then
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
case_ readme_indented_block_added fail "indented line under \"Install\"" \
    edit_readme '/^The icon appears in the tray\.$/a\
\
\    echo indented'
case_ readme_space_tab_indented_line fail "indented line under \"Install\"" \
    edit_readme 's/^The icon appears in the tray\.$/&\
\
 \techo stray/'
case_ readme_indented_fences fail "indented line under \"Install\"" \
    edit_readme 's/^```/    ```/'
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
# Each fails one query only: the shape check, which would say it could not
# run, still runs.
jq_failed="jq failed reading docs/facts.json|!shape check could not run"
case_ jq_fails_on_commands fail "$jq_failed" failing_jq 'select(.status'
case_ jq_fails_on_command_lines fail "$jq_failed|!README.md's commands" failing_jq '.command // empty'
case_ jq_fails_on_hotkeys fail "$jq_failed|!hotkey table" failing_jq 'hotkeys.modifiers as'
case_ jq_fails_on_paths fail "$jq_failed" failing_jq '.links[]'
case_ jq_fails_on_quotes fail "$jq_failed" failing_jq 'source.quote'
case_ nothing_available pass "matches README.md" \
    edit_facts '.platforms[0] |= (.status = "soon" | .lines = [] | del(.readme) | del(.note))'
case_ nul_in_file fail "not valid JSON" put_nul_in_facts
case_ two_json_values fail "one value, and nothing else" two_facts_values
case_ nul_in_readme fail "README.md contains a NUL byte" \
    edit_readme 's/git clone/git clo\x00ne/' 
if ln -s README.md "$work/probe" 2>/dev/null && [[ -L "$work/probe" ]]; then
    case_ path_is_a_link fail "docs/guide/install.md, which is not a file" link_install_guide
    case_ readme_is_a_link fail "README.md is not a file in the repository" link_readme
    case_ facts_is_a_link fail "docs/facts.json is not a file in the repository" link_facts
    case_ path_through_a_linked_directory fail "docs/linked/install.md, which is not a file" \
        link_guide_directory
else
    echo "skip path_is_a_link, path_through_a_linked_directory: this system makes no symbolic links"
fi
case_ quote_missing fail "figure idle-memory: docs/planning/map.md does not contain" \
    edit_facts '.figures[2].source.quote = "idles at 9.9 MiB"'

if [[ "$failures" -gt 0 ]]; then
    echo "$failures check-facts case(s) failed" >&2
    exit 1
fi
echo "all check-facts cases passed"
