# The display shows both pictures under a lock: driven through a FIFO so it
# outlives this SSH command. CMD is one of start, lock, send, stop.
cd /tmp/o01a
export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
case $CMD in
start)
  rm -f ctl; mkfifo ctl
  setsid -f bash -c 'sleep 900 > /tmp/o01a/ctl 2>/dev/null & exec target/release/o01-probe show < /tmp/o01a/ctl > /tmp/o01a/lock-probe.log 2>&1' < /dev/null > /dev/null 2>&1
  sleep 0.5; echo "both 9" > ctl; sleep 1; grep -E "drew|entered" lock-probe.log | cut -c1-80 ;;
lock)
  sid=$(loginctl list-sessions --no-legend | awk '$4=="seat0"{print $1}'); loginctl lock-session $sid; sleep 2; loginctl show-session $sid -p LockedHint ;;
send) echo "$WHAT" > ctl; sleep 0.5; tail -3 lock-probe.log ;;
stop) echo quit > ctl; sleep 0.5; pkill -f "sleep 900"; cat lock-probe.log | grep -v preferred | tail -25 ;;
esac
