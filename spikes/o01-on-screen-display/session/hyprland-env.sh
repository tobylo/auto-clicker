# The Hyprland session's environment, from its own process.
pid=$(pgrep -x Hyprland | head -1)
while IFS= read -r -d '' kv; do case "$kv" in WAYLAND_DISPLAY=*|DISPLAY=*|XAUTHORITY=*|XDG_*|HYPRLAND_*|DBUS_SESSION_BUS_ADDRESS=*|PATH=*|HOME=*|USER=*) export "$kv";; esac; done < /proc/$pid/environ
export XDG_RUNTIME_DIR=/run/user/1000 HYPRLAND_INSTANCE_SIGNATURE=$(ls -t /run/user/1000/hypr | head -1)
export WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1}
[ -z "$DISPLAY" ] && export DISPLAY=$(hyprctl -j getoption xwayland:enabled >/dev/null 2>&1; echo :0)
