<img src="assets/icon.svg" width="64" height="64" alt="">

# auto-clicker

An auto clicker with macro record and playback for KDE Plasma on Wayland, living in the system tray. Written in Rust to stay light while idle.

Status: planning. See [docs/planning/map.md](docs/planning/map.md) for what it will do and why, and [the Project](https://github.com/users/tobylo/projects/4) for the work.

## Configuration

Settings live in `~/.config/auto-clicker/config.toml` (or under `$XDG_CONFIG_HOME`). Without the file, the defaults below apply; the program never writes it. If the file is invalid, the program runs on the defaults, leaves the file alone and reports the line and reason.

```toml
version = 1
tray_start_delay = 5          # seconds before "Start clicking" in the tray clicks; 0 to 3600

[clicker]
mode = "click"                # click, double_click, hold_button, key_repeat or hold_key
interval_ms = 100             # 10 or more (20 for double_click); or clicks_per_second = 10, not both
jitter_percent = 0            # vary each interval randomly by up to this much, 0 to 50
button = "left"               # left, right, middle, back or forward
key = "space"                 # for key_repeat and hold_key
position = "cursor"           # or a fixed point, { x = 32768, y = 32768 }
repeat = "until_stopped"      # or a number of clicks
```

- Key names are the Linux names in lowercase without `KEY_`: "a" to "z", "0" to "9", "f1" to "f24", "space", "enter", "esc", "tab", "leftshift", "leftctrl", "up", and so on. They name positions on a US keyboard, not the label on the key: on a German keyboard, "z" is the key labelled Y. A key without a name can be given by its key code, such as `key = 84`.
- A fixed point runs from 0 to 65535 across all screens together, left to right and top to bottom. Games that lock the pointer (Minecraft while playing) ignore it; use the cursor position there.
