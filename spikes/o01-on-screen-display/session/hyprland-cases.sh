cd /tmp/o01a
systemctl --user stop auto-clicker.service
trap 'systemctl --user start auto-clicker.service' EXIT
for case in $CASES; do
  timeout 30 python3 run.py ${case//:/ } 2>&1
  sleep 1
done
