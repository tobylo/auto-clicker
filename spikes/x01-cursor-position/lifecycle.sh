#!/bin/bash
# X01 lifecycle checks. Run under the shared spike lock.
#  1. A receiver killed with SIGKILL leaves the script loaded; the next start
#     unloads it.
#  2. KWin restarting drops scripts loaded over D-Bus. Checked on a nested,
#     virtual KWin in its own D-Bus session, never on the live one.
set -u
cd "$(dirname "$0")"
loaded() { qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.isScriptLoaded x01-cursor; }

echo "== receiver crash"
md5sum ~/.config/kwinrc
python3 harness/x01.py receive --seconds 60 &
receiver=$!
sleep 2
kill -9 "$receiver"; wait "$receiver" 2>/dev/null
echo "after SIGKILL, script loaded: $(loaded)"
python3 harness/x01.py layout | head -1
echo "after the next start, script loaded: $(loaded)"
md5sum ~/.config/kwinrc

echo "== KWin restart (nested, virtual backend, private bus)"
dbus-run-session -- bash -c '
    start() {
        env -u WAYLAND_DISPLAY -u DISPLAY kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
            --socket x01-nested --width 1280 --height 800 >/dev/null 2>&1 &
        kwin=$!
        for _ in $(seq 50); do qdbus6 org.kde.KWin /Scripting >/dev/null 2>&1 && return; sleep 0.1; done
        echo "nested KWin did not start"; exit 1
    }
    owner() { qdbus6 org.freedesktop.DBus / org.freedesktop.DBus.GetNameOwner org.kde.KWin 2>/dev/null; }
    start
    id=$(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$PWD/kwin/x01-cursor.js" x01-cursor)
    qdbus6 org.kde.KWin /Scripting/Script$id org.kde.kwin.Script.run
    echo "nested KWin $(owner): script loaded: $(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.isScriptLoaded x01-cursor)"
    kill $kwin; wait $kwin 2>/dev/null
    echo "after KWin exit, org.kde.KWin owner: $(owner || true)"
    start
    echo "restarted KWin $(owner): script loaded: $(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.isScriptLoaded x01-cursor)"
    kill $kwin; wait $kwin 2>/dev/null
'
