# The Hyprland session's environment, from its own process: the newest one,
# and the instance whose lock file names that process.
pid=$(pgrep -nx Hyprland)
while IFS= read -r -d '' kv; do case "$kv" in WAYLAND_DISPLAY=*|DISPLAY=*|XAUTHORITY=*|XDG_*|HYPRLAND_*|DBUS_SESSION_BUS_ADDRESS=*|PATH=*|HOME=*|USER=*) export "$kv";; esac; done < /proc/$pid/environ
for dir in /run/user/1000/hypr/*/; do [ "$(head -1 "$dir/hyprland.lock" 2>/dev/null)" = "$pid" ] && signature=$(basename "$dir"); done
export XDG_RUNTIME_DIR=/run/user/1000 HYPRLAND_INSTANCE_SIGNATURE=$signature
export WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1}
[ -z "$DISPLAY" ] && export DISPLAY=$(hyprctl -j getoption xwayland:enabled >/dev/null 2>&1; echo :0)
