# The display shows both pictures under a lock, driven through a FIFO so it
# outlives this SSH command. CMD is start, lock, send (WHAT=command) or stop.
# The display opens the FIFO read-write itself, so it never sees an end of
# file and needs no other process to hold it open; every write to the FIFO
# gives up after 2 s, so a dead display cannot block.
cd /tmp/o01a || exit 1
export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
say() { timeout 2 bash -c 'echo "$1" > /tmp/o01a/ctl' _ "$1" || { echo "the display did not take: $1"; return 1; }; }
case $CMD in
start)
  # A FIFO left means a display may still run on it: stop that one first,
  # or it is left showing with no way to reach it.
  [ -e ctl ] && { echo "/tmp/o01a/ctl exists: run CMD=stop first"; exit 1; }
  mkfifo ctl || exit 1
  setsid -f bash -c 'exec target/release/o01-probe show <> /tmp/o01a/ctl > /tmp/o01a/lock-probe.log 2>&1' < /dev/null > /dev/null 2>&1
  sleep 0.5; say "both 9"; sleep 1; grep -E "drew|entered" lock-probe.log | cut -c1-80 ;;
lock)
  sid=$(loginctl list-sessions --no-legend | awk '$4=="seat0"{print $1}'); loginctl lock-session "$sid"; sleep 2; loginctl show-session "$sid" -p LockedHint ;;
send) say "$WHAT"; sleep 0.5; tail -3 lock-probe.log ;;
stop)
  # The FIFO goes only once the display has quit; otherwise it stays, so a
  # later stop can try again and a start still refuses.
  say quit; sleep 0.5
  # Whether quit arrived or not (a dead display cannot take it), the FIFO
  # goes once no display is left to read it.
  pgrep -f "o01-probe show" >/dev/null || rm -f ctl
  [ -e ctl ] && echo "the display has not quit; /tmp/o01a/ctl kept"
  grep -v preferred lock-probe.log | tail -25 ;;
esac
