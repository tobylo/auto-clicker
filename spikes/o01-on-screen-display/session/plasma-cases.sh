# Runs run.py cases on the laptop's Plasma session; the daemon is stopped
# around them and started again whatever happens.
cd /tmp/o01a || exit 1
export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus DISPLAY=:0
export XAUTHORITY=$(ls /run/user/1000/xauth_* | head -1)
systemctl --user stop auto-clicker.service
trap 'systemctl --user start auto-clicker.service' EXIT
kscreen-doctor --dpms on >/dev/null 2>&1; sleep 1
for case in $CASES; do
  timeout 30 python3 run.py ${case//:/ } 2>&1
  sleep 1
done
