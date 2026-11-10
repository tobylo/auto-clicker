#!/bin/bash
# X01 lifecycle checks. Run under the shared spike lock.
#  1. A receiver killed with SIGKILL leaves the script loaded; the next start
#     unloads it.
#  2. KWin restarting drops scripts loaded over D-Bus. Checked on a nested,
#     virtual KWin in its own D-Bus session, never on the live one.
# Any unexpected result ends the run with status 1. Whatever happens, the
# exit trap stops the receiver and unloads the script from the live KWin.
set -u
cd "$(dirname "$0")"
live_kwin=$(qdbus6 org.freedesktop.DBus / org.freedesktop.DBus.GetNameOwner org.kde.KWin) || exit 1
loaded() { qdbus6 "$live_kwin" /Scripting org.kde.kwin.Scripting.isScriptLoaded x01-cursor; }
receiver=
cleanup() {
    if [ -n "$receiver" ]; then kill -9 "$receiver" 2>/dev/null; wait "$receiver" 2>/dev/null; fi
    qdbus6 "$live_kwin" /Scripting org.kde.kwin.Scripting.unloadScript x01-cursor >/dev/null 2>&1
}
trap cleanup EXIT
expect() {
    echo "$1: $2"
    [ "$2" = "$3" ] || { echo "unexpected: $1 should be $3"; exit 1; }
}

echo "== receiver crash"
md5sum ~/.config/kwinrc
python3 harness/x01.py receive --seconds 60 &
receiver=$!
sleep 2
kill -9 "$receiver"; wait "$receiver" 2>/dev/null; receiver=
expect "after SIGKILL, script loaded" "$(loaded)" true
next_start=$(python3 harness/x01.py layout) || { echo "the next start failed"; exit 1; }
head -1 <<<"$next_start"
expect "after the next start, script loaded" "$(loaded)" false
md5sum ~/.config/kwinrc

echo "== KWin restart (nested, virtual backend, private bus and private config)"
# A virtual KWin writes kwinrc (desktops, tiling for its virtual output), so
# it gets its own config, data, cache and state directories, never the
# user's; they are removed afterwards.
nested_home=$(mktemp -d) || exit 1
trap 'cleanup; rm -rf "$nested_home"' EXIT
mkdir -p "$nested_home"/{config,data,cache,state}
XDG_CONFIG_HOME=$nested_home/config XDG_DATA_HOME=$nested_home/data \
XDG_CACHE_HOME=$nested_home/cache XDG_STATE_HOME=$nested_home/state \
dbus-run-session -- bash -c '
    start() {
        env -u WAYLAND_DISPLAY -u DISPLAY kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
            --socket x01-nested --width 1280 --height 800 >/dev/null 2>&1 &
        kwin=$!
        for _ in $(seq 50); do qdbus6 org.kde.KWin /Scripting >/dev/null 2>&1 && return; sleep 0.1; done
        echo "nested KWin did not start"; exit 1
    }
    owner() { qdbus6 org.freedesktop.DBus / org.freedesktop.DBus.GetNameOwner org.kde.KWin 2>/dev/null; }
    stop() {
        [ -n "${kwin:-}" ] || return 0
        kill $kwin 2>/dev/null
        for _ in $(seq 50); do kill -0 $kwin 2>/dev/null || break; sleep 0.1; done
        kill -9 $kwin 2>/dev/null
        wait $kwin 2>/dev/null
        kwin=
    }
    trap stop EXIT
    start
    id=$(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$PWD/kwin/x01-cursor.js" x01-cursor)
    qdbus6 org.kde.KWin /Scripting/Script$id org.kde.kwin.Script.run
    state=$(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.isScriptLoaded x01-cursor)
    echo "nested KWin $(owner): script loaded: $state"
    [ "$state" = true ] || exit 1
    stop
    echo "after KWin exit, org.kde.KWin owner: $(owner || true)"
    start
    state=$(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.isScriptLoaded x01-cursor)
    echo "restarted KWin $(owner): script loaded: $state"
    [ "$state" = false ] || exit 1
    stop
' || { echo "nested KWin check failed"; exit 1; }
md5sum ~/.config/kwinrc
