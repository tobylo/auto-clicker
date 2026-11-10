<img src="assets/icon.svg" width="64" height="64" alt="">

# auto-clicker

An auto clicker with macro record and playback for KDE Plasma on Wayland, living in the system tray. Written in Rust to stay light while idle.

Status: the auto clicker runs from the tray and the hotkey; installing, recording and playback are still to come. See [docs/planning/map.md](docs/planning/map.md) for what it will do and why, and [the Project](https://github.com/users/tobylo/projects/4) for the work.

## Using it

Run `auto-clicker`. It stays in the system tray; a left or right click on its icon opens the menu:

- **Start clicking** counts down `tray_start_delay` seconds (5 by default), so you can switch to the game, then clicks. The icon's tooltip shows the seconds left. **Stop clicking** stops it, or cancels the countdown.
- **Record**, **Play** and **Settings** come in later versions.
- **Quit** stops clicking, lets go of everything it pressed and exits.

The hotkey (Ctrl+F6 by default) starts and stops clicking at once, without a countdown. A start waits until you let go of Ctrl, so the first click is a plain click and not Ctrl+click; if Ctrl is still held after 30 seconds, nothing starts and the tooltip says why.

Clicking stops when the screen locks, another user's session comes to the front, or the computer goes to sleep, and it cannot start again until the session is back. The tooltip says what the clicker will do, and why it stopped or cannot start.

Only one copy runs per session: a second one prints "auto-clicker is already running" and exits.

Exit statuses: 0 after Quit, SIGTERM or SIGINT; 3 when another copy is already running; 77 when `/dev/uinput` cannot be used (no permission, or the `uinput` module is not loaded), which restarting does not fix; 1 for any other failure at start.

## Configuration

Settings live in `~/.config/auto-clicker/config.toml` (or under `$XDG_CONFIG_HOME`). Without the file, the defaults below apply; the program never writes it. If the file is invalid, the program runs on the defaults, leaves the file alone and reports the line and reason in the tray's tooltip. The file is read at start only: restart the program after editing it.

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
- A hotkey is any of Ctrl, Shift, Alt and Meta, then a key name from the list above, joined by `+`, in any letter case: "Ctrl+F6", "ctrl+shift+f9", "F13". It fires when the key is pressed while exactly those modifiers are held, left or right alike, on any keyboard. Two actions cannot share a hotkey, and no hotkey can use the key the clicker presses.

## Hotkeys

Hotkeys work in every window, fullscreen games included, because the program reads the keyboards directly. That needs read access to `/dev/input`: the user must be in the `input` group. When no keyboard can be read, the program keeps running and the tray says why.

The keyboards are only read, never taken over, so the game also gets the hotkey. In Minecraft, Ctrl is sprint: pressing Ctrl+F6 also starts sprinting for a moment. Pick a hotkey the game ignores if that gets in the way.

If a key remapper (keyd, kanata) takes over a keyboard while Ctrl or another modifier is held on it, the program never sees that key come up, and hotkeys stay blocked until the keyboard is unplugged or the program restarts.

A run that holds a modifier itself (`hold_key` with `key = "leftshift"`, say) and a remapper that re-sends this program's virtual keyboard (keyd's default, which takes over every keyboard) do not mix: the hotkey then arrives with that modifier added, does not match, and cannot stop the run. Stop it from the tray, or set the remapper to leave the "auto-clicker virtual keyboard" device alone.
