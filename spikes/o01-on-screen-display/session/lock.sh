# The display shows both pictures under a lock, driven through a FIFO so it
# outlives this SSH command. CMD is start, lock, send (WHAT=command) or stop.
# The FIFO's keeper is this run's own process, remembered by its PID; every
# write to the FIFO gives up after 2 s, so a dead display cannot block.
cd /tmp/o01a || exit 1
export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
say() { timeout 2 bash -c 'echo "$1" > /tmp/o01a/ctl' _ "$1" || echo "the display did not take: $1"; }
case $CMD in
start)
  rm -f ctl keeper.pid; mkfifo ctl
  setsid -f bash -c 'sleep 900 > /tmp/o01a/ctl 2>/dev/null & echo $! > /tmp/o01a/keeper.pid; exec target/release/o01-probe show < /tmp/o01a/ctl > /tmp/o01a/lock-probe.log 2>&1' < /dev/null > /dev/null 2>&1
  sleep 0.5; say "both 9"; sleep 1; grep -E "drew|entered" lock-probe.log | cut -c1-80 ;;
lock)
  sid=$(loginctl list-sessions --no-legend | awk '$4=="seat0"{print $1}'); loginctl lock-session "$sid"; sleep 2; loginctl show-session "$sid" -p LockedHint ;;
send) say "$WHAT"; sleep 0.5; tail -3 lock-probe.log ;;
stop)
  say quit; sleep 0.5
  [ -s keeper.pid ] && kill "$(cat keeper.pid)" 2>/dev/null
  rm -f ctl keeper.pid; grep -v preferred lock-probe.log | tail -25 ;;
esac
