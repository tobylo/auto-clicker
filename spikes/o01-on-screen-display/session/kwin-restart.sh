# A nested, virtual KWin under kwin_wayland_wrapper (which keeps the socket
# and restarts KWin, as in a Plasma session); the display shows on it, then
# the KWin process is killed. Every probe has a 5 s limit, and the wrapper
# is stopped on every way out. Leaves nothing running.
cd /tmp/o01a || exit 1
export XDG_RUNTIME_DIR=/run/user/1000
dbus-run-session -- bash -c '
  env -u WAYLAND_DISPLAY -u DISPLAY kwin_wayland_wrapper --virtual --no-lockscreen --no-global-shortcuts \
      --width 1920 --height 1080 >/tmp/o01a/nested.log 2>&1 &
  wrapper=$!
  trap "kill \$wrapper 2>/dev/null; wait \$wrapper 2>/dev/null" EXIT
  for _ in $(seq 50); do kwin=$(pgrep -P $wrapper -x kwin_wayland) && break; sleep 0.2; done
  name=$(tr "\0" "\n" < /proc/$kwin/cmdline | grep -A1 -x -- --socket | sed -n 2p); echo "nested socket $name"
  for _ in $(seq 50); do WAYLAND_DISPLAY=$name timeout 5 target/release/o01-probe outputs >/dev/null 2>&1 && break; sleep 0.2; done
  ( echo "both 5"; sleep 3; echo "both 4"; sleep 4; echo "both 3"; sleep 1; echo quit ) | WAYLAND_DISPLAY=$name timeout 15 target/release/o01-probe show > nested-probe.log 2>&1 &
  probe=$!
  sleep 1.5
  kwin=$(pgrep -P $wrapper -x kwin_wayland); echo "killing kwin_wayland $kwin (SIGKILL)"; kill -KILL $kwin
  sleep 3
  echo "after restart: new kwin $(pgrep -P $wrapper -x kwin_wayland)"; WAYLAND_DISPLAY=$name timeout 5 target/release/o01-probe outputs
  wait $probe; echo "probe exit $?"; cat nested-probe.log | grep -v preferred
'
pgrep -af "kwin_wayland_wrapper --virtual" || echo "nothing left running"
