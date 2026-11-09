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
repeat = "until_stopped"      # or a number of actions (a double click is one); hold modes run until stopped

[hotkeys]
toggle_clicking = "Ctrl+F6"   # start or stop clicking
toggle_recording = "Ctrl+F7"  # start or stop recording a macro
toggle_playback = "Ctrl+F8"   # play or stop the macro
```

- Keys are named by position on a US keyboard, not by the label on the key: on a German keyboard, "z" is the key labelled Y. The names, lowercase: "a" to "z", "0" to "9", "f1" to "f24", "space", "enter", "esc", "tab", "backspace", "capslock", "minus", "equal", "leftbrace", "rightbrace", "semicolon", "apostrophe", "grave", "backslash", "comma", "dot", "slash", left and right "shift", "ctrl", "alt" and "meta" (such as "leftshift"), "up", "down", "left", "right", "home", "end", "pageup", "pagedown", "insert", "delete", the keypad keys ("kp0" to "kp9", "kpplus", "kpminus", "kpasterisk", "kpslash", "kpdot", "kpenter"), "numlock", "scrolllock", "sysrq", "pause" and "compose". Any other key is given by its Linux key code, such as `key = 86`.
- A fixed point runs from 0 to 65535 across all screens together, left to right and top to bottom. Games that lock the pointer (Minecraft while playing) ignore it; use the cursor position there.
- A hotkey is any of Ctrl, Shift, Alt and Meta, then a key name from the list above, joined by `+`, in any letter case: "Ctrl+F6", "ctrl+shift+f9", "F13". It fires when the key is pressed while exactly those modifiers are held, left or right alike, on any keyboard. Two actions cannot share a hotkey, and a hotkey without a modifier cannot be the key the clicker presses.

## Hotkeys

Hotkeys work in every window, fullscreen games included, because the program reads the keyboards directly. That needs read access to `/dev/input`: the user must be in the `input` group. When no keyboard can be read, the program keeps running and the tray says why.

The keyboards are only read, never taken over, so the game also gets the hotkey. In Minecraft, Ctrl is sprint: pressing Ctrl+F6 also starts sprinting for a moment. Pick a hotkey the game ignores if that gets in the way.

If a key remapper (keyd, kanata) takes over a keyboard while Ctrl or another modifier is held on it, the program never sees that key come up, and hotkeys stay blocked until the keyboard is unplugged or the program restarts.
