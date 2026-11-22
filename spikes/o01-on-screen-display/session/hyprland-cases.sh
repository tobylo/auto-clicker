# Runs run.py cases on the laptop's Hyprland session (after hyprland-env.sh); the
# daemon is stopped around them and started again whatever happens.
cd /tmp/o01a || exit 1
systemctl --user stop auto-clicker.service
trap 'systemctl --user start auto-clicker.service' EXIT
for case in $CASES; do
  timeout 30 python3 run.py ${case//:/ } 2>&1
  sleep 1
done
