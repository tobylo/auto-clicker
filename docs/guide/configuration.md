# Configuration

Settings live in `~/.config/auto-clicker/config.toml` (or under `$XDG_CONFIG_HOME`). Without the file, the defaults below apply. The settings window manages the file: a save writes all of it anew, with every setting and without comments, so edit settings in the window, or edit the file and leave the window closed. A `config.toml` that is a symbolic link is saved through the link. If the file is invalid, the program runs on the defaults, leaves the file alone and reports the line and reason in the tray's tooltip. The program reads the file at start; after editing it by hand, restart the program (`systemctl --user restart auto-clicker`).

```toml
version = 1
tray_start_delay = 5          # seconds before "Start clicking" in the tray clicks; 0 to 3600

[clicker]
mode = "click"                # click, double_click, hold_button, key_repeat or hold_key
interval_ms = 100             # 10 or more (20 for double_click); or clicks_per_second = 10, not both
jitter_percent = 0            # vary each interval randomly by up to this much, 0 to 50
button = "left"               # left, right, middle, back or forward
key = "space"                 # for key_repeat and hold_key
position = "cursor"           # or a fixed point, { x = 32768, y = 32768 }, with the screens it was picked on
repeat = "until_stopped"      # or a number of actions (a double click is one); hold modes run until stopped

[hotkeys]
toggle_clicking = "Ctrl+Shift+F6"   # start or stop clicking
toggle_recording = "Ctrl+Shift+F7"  # start recording a macro, or stop and save it
toggle_playback = "Ctrl+Shift+F8"   # play the current recording, or stop playback

[input]
output = "auto"               # how clicks and keys reach the desktop: auto, kernel or portal
hotkeys = "auto"              # where hotkeys come from: auto, kernel or portal

[playback]
speed = 1.0                   # how fast a recording plays, 0.25 to 4 times the recorded speed
repeat = 1                    # how many times it plays, or "until_stopped"
```

- Keys are named by where they sit on the keyboard, not by the label printed on them, so a setting stays on the same physical key whatever keyboard layout is in use. The letter and symbol names are the labels of a US keyboard: on a German keyboard, "z" is the key labelled Y. The names, lowercase: "a" to "z", "0" to "9", "f1" to "f24", "space", "enter", "esc", "tab", "backspace", "capslock", "minus", "equal", "leftbrace", "rightbrace", "semicolon", "apostrophe", "grave", "backslash", "comma", "dot", "slash", left and right "shift", "ctrl", "alt" and "meta" (such as "leftshift"), "up", "down", "left", "right", "home", "end", "pageup", "pagedown", "insert", "delete", the keypad keys ("kp0" to "kp9", "kpplus", "kpminus", "kpasterisk", "kpslash", "kpdot", "kpenter"), "numlock", "scrolllock", "sysrq", "pause", "compose" and "102nd", the extra key beside the left Shift on ISO keyboards (most European ones). Any other key is given by its Linux key code, such as `key = 113` for the mute key.
- On a Swedish or Finnish keyboard, the letters, the digits, "comma" and "dot" match their labels. The other keys: "grave" is §, "minus" is +, "equal" is ´, "leftbrace" is Å, "rightbrace" is ¨, "semicolon" is Ö, "apostrophe" is Ä, "backslash" is ', "102nd" is < and "slash" is -.
- A fixed point runs from 0 to 65535 across all screens together, left to right and top to bottom. Games that lock the pointer (Minecraft while playing) ignore it; use the cursor position there. A point picked in the settings window also has `screens`, the screens it was picked on, as a recording keeps them: `{ x = 32768, y = 32768, screens = [{ x = 0, y = 0, width = 2048, height = 1280, scale_milli = 1250 }] }`. With `screens`, a click mode starts only while the screens are in those places and sizes, and stops when they change; without it, the point is used on any screens.
- `[input]` chooses the route clicks and hotkeys take; see [Choosing the route](routes.md#choosing-the-route).

- A hotkey is any of Ctrl, Shift, Alt and Meta, then a key name from the list above, joined by `+`, in any letter case: "Ctrl+F6", "ctrl+shift+f9", "F13". It fires when the key is pressed while exactly those modifiers are held, left or right alike, on any keyboard. Two actions cannot share a hotkey, and with `hotkeys = "kernel"` or `"auto"` no hotkey can use the key the clicker presses (with `hotkeys = "portal"`, Plasma's own hotkeys decide that).
