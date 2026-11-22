# Starts Minecraft 26.3 without the launcher, with MangoHud logging every
# frame for LOGSECS s (240 by default). DRIVER (wayland or x11) and DIR, a
# plain name for the log directory under /tmp/o01a, come from the caller.
pid=$(pgrep -u tobylo -x plasmashell | head -1)
while IFS= read -r -d '' kv; do export "$kv"; done < /proc/$pid/environ
pgrep -f java-runtime-epsilon && { echo "a game is still running"; exit 1; }
case "$DIR" in ''|*/*|.|..) echo "DIR must be a plain name"; exit 1 ;; esac
[ -e "/tmp/o01a/$DIR" ] && { echo "/tmp/o01a/$DIR exists already; pick a new DIR"; exit 1; }
mkdir -p "/tmp/o01a/$DIR"
export MANGOHUD_CONFIG="output_folder=/tmp/o01a/${DIR},autostart_log=1,log_duration=${LOGSECS:-240},log_interval=0,fps_only,position=bottom-right"
export SDL_VIDEO_DRIVER=$DRIVER SDL_VIDEODRIVER=$DRIVER
setsid -f mangohud python3 /tmp/o01a/mclaunch.py 26.3 >"/tmp/o01a/game-$DIR.log" 2>&1 < /dev/null
sleep 12; grep -E "video driver" "/tmp/o01a/game-$DIR.log"; date +%T
