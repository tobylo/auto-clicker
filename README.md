<img src="assets/icon.svg" width="64" height="64" alt="">

# auto-clicker

An auto clicker with macro record and playback for KDE Plasma on Wayland, living in the system tray. Written in Rust to stay light while idle.

Status: the auto clicker runs from the tray and the hotkey, and installs as a package on CachyOS and Arch; recording and playback are still to come. See [docs/planning/map.md](docs/planning/map.md) for what it will do and why, and [the Project](https://github.com/users/tobylo/projects/4) for the work.

## Install

These steps are for CachyOS (or Arch Linux) with Plasma. The computer builds the program itself from this repository; nothing else is downloaded except the Rust libraries it uses, from crates.io, at the exact versions in `Cargo.lock`. Each command is one line; paste it into a terminal (Konsole).

### Once per computer

Run these from an account that can use `sudo`.

1. Bring the system up to date, then restart the computer if the update included a new kernel:
   `sudo pacman -Syu`
2. Install the build tools:
   `sudo pacman -S --needed base-devel git`
3. Download this repository:
   `git clone https://github.com/tobylo/auto-clicker.git ~/auto-clicker`
4. Build and install the package. This takes a few minutes, installs Rust the first time, and asks for your password:
   `cd ~/auto-clicker/packaging/arch && makepkg -sif`

The package is called `tobylo-auto-clicker`. It installs the program, its tray icons, an "Auto Clicker" entry in the app menu, a background service, and a rule that lets the person at the screen use `/dev/uinput`.

### For each person who uses it

1. Let them read the keyboard for the hotkeys (replace `NAME` with their login name):
   `sudo usermod -aG input NAME`
2. Restart the computer, so the new group and the `/dev/uinput` rule take effect.
3. Logged in as that person, start it now and at every login:
   `systemctl --user enable --now auto-clicker`

The icon then appears in the system tray. After **Quit**, start it again from "Auto Clicker" in the app menu, or log in again.

### What the permissions mean

- **`/dev/uinput`** lets a program create a virtual mouse and keyboard, which is how auto-clicker clicks. The rule lets programs of the person logged in at the screen open it, any of their programs, not just this one. When someone else takes over the screen, no new program of the first person can open it, but one that already has it open keeps it. auto-clicker itself stops clicking and lets go of everything when the screen locks or another user's session comes to the front.
- **The `input` group** lets a program read every keyboard and mouse directly, so the hotkey works inside fullscreen games. auto-clicker only looks for its hotkeys, and records keys only while you record a macro. But the group applies to every program that person runs, not just this one: only add people whose programs you trust.
- The background service may use local Unix sockets only, so the program cannot reach the network even by mistake. The program itself opens no network connection.

### Updating

The program does not update itself. To install a newer version, from the account that installed it:

1. Fetch the new commits and list them:
   `cd ~/auto-clicker && git fetch && git log --oneline HEAD..origin/main`
2. Check that each one is a change you reviewed and merged on GitHub. Don't go on if one is not.
3. Bring them in, then build and install:
   `cd ~/auto-clicker/packaging/arch && git merge --ff-only origin/main && makepkg -sif`

Then restart the computer, or, for each person, `systemctl --user restart auto-clicker`. Always use `-f`: without it, makepkg would reinstall the package it built last time.

The build refuses to run if the downloaded copy has been changed by hand (`git status` shows changes), so what gets installed is always a version from this repository.

### If it does not start

`systemctl --user status auto-clicker` shows whether it runs, and `journalctl --user -b -u auto-clicker` shows its messages. Exit status 77 means it cannot use `/dev/uinput`: restart the computer after installing. Exit status 3 means another copy is already running.

### Uninstall

1. For each person who enabled it, logged in as them:
   `systemctl --user disable --now auto-clicker`
2. Remove the package:
   `sudo pacman -Rns tobylo-auto-clicker`
3. Take each person out of the `input` group, unless something else needs it, then restart the computer: programs already running keep the group until then.
   `sudo gpasswd -d NAME input`
4. Optionally, from the account that installed it, delete the downloaded copy and Rust, which the build installed:
   `rm -rf ~/auto-clicker; sudo pacman -Rns rust`
5. Optionally, as each person, delete their settings:
   `rm -rf ~/.config/auto-clicker`

## Using it

Once installed and enabled, it starts at login and stays in the system tray (run `auto-clicker` to start it by hand). A left or right click on its icon opens the menu:

- **Start clicking** counts down `tray_start_delay` seconds (5 by default), so you can switch to the game, then clicks. The icon's tooltip shows the seconds left. **Stop clicking** stops it, or cancels the countdown.
- **Record**, **Play** and **Settings** come in later versions.
- **Quit** stops clicking, lets go of everything it pressed and exits.

The hotkey (Ctrl+F6 by default) starts and stops clicking at once, without a countdown. A start waits until you let go of Ctrl, so the first click is a plain click and not Ctrl+click; if Ctrl is still held after 30 seconds, nothing starts and the tooltip says why.

Clicking stops when the screen locks, another user's session comes to the front, or the computer goes to sleep, and it cannot start again until the session is back. Before sleep, the program holds the computer back (for 2 seconds at most) until it has let go of every button and key it pressed, so nothing stays pressed while asleep. The tooltip says what the clicker will do, and why it stopped or cannot start.

Only one copy runs per session: a second one prints "auto-clicker is already running" and exits. A running copy quits if it loses the session bus. It talks to the session and system buses through their Unix sockets only, and refuses to start when `DBUS_SESSION_BUS_ADDRESS` names anything else.

Exit statuses: 0 after Quit, SIGTERM or SIGINT; 3 when another copy is already running; 77 when `/dev/uinput` cannot be used (no permission, or the `uinput` module is not loaded), which restarting does not fix; 1 for any other failure at start.

## Configuration

Settings live in `~/.config/auto-clicker/config.toml` (or under `$XDG_CONFIG_HOME`). Without the file, the defaults below apply; the program never writes it. If the file is invalid, the program runs on the defaults, leaves the file alone and reports the line and reason in the tray's tooltip. The file is read at start only: after editing it, restart the program (`systemctl --user restart auto-clicker`).

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

[input]
output = "kernel"             # how clicks and keys reach the desktop: kernel or portal
```

- Keys are named by where they sit on the keyboard, not by the label printed on them, so a setting stays on the same physical key whatever keyboard layout is in use. The letter and symbol names are the labels of a US keyboard: on a German keyboard, "z" is the key labelled Y. The names, lowercase: "a" to "z", "0" to "9", "f1" to "f24", "space", "enter", "esc", "tab", "backspace", "capslock", "minus", "equal", "leftbrace", "rightbrace", "semicolon", "apostrophe", "grave", "backslash", "comma", "dot", "slash", left and right "shift", "ctrl", "alt" and "meta" (such as "leftshift"), "up", "down", "left", "right", "home", "end", "pageup", "pagedown", "insert", "delete", the keypad keys ("kp0" to "kp9", "kpplus", "kpminus", "kpasterisk", "kpslash", "kpdot", "kpenter"), "numlock", "scrolllock", "sysrq", "pause", "compose" and "102nd", the extra key beside the left Shift on ISO keyboards (most European ones). Any other key is given by its Linux key code, such as `key = 113` for the mute key.
- On a Swedish or Finnish keyboard, the letters, the digits, "comma" and "dot" match their labels. The other keys: "grave" is §, "minus" is +, "equal" is ´, "leftbrace" is Å, "rightbrace" is ¨, "semicolon" is Ö, "apostrophe" is Ä, "backslash" is ', "102nd" is < and "slash" is -.
- A fixed point runs from 0 to 65535 across all screens together, left to right and top to bottom. Games that lock the pointer (Minecraft while playing) ignore it; use the cursor position there.
- `output = "portal"` clicks and types without `/dev/uinput`, through the desktop's remote input portal (see [Clicking through the portal](#clicking-through-the-portal)). `kernel`, the default, uses `/dev/uinput`. Hotkeys still read the keyboards directly either way.
- A hotkey is any of Ctrl, Shift, Alt and Meta, then a key name from the list above, joined by `+`, in any letter case: "Ctrl+F6", "ctrl+shift+f9", "F13". It fires when the key is pressed while exactly those modifiers are held, left or right alike, on any keyboard. Two actions cannot share a hotkey, and no hotkey can use the key the clicker presses.

## Clicking through the portal

With `output = "portal"`, the program asks Plasma for remote input (the RemoteDesktop portal) instead of creating virtual devices, so it needs no access to `/dev/uinput`. The first time you start clicking, Plasma asks whether "Auto Clicker" may control the keyboard and pointer. After you allow it, clicking does not begin by itself, since the dialog took the focus from your game: switch back and start again. Plasma remembers the answer, so later starts go ahead without asking; the program keeps Plasma's token for it in `~/.local/state/auto-clicker/portal-token`. If you deny it, the tray says so and the next start asks again. Plasma lists the permission under System Settings, where it can be revoked.

The program opens a remote input session for each run and closes it when the run stops, so Plasma shows that remote control is active only while clicking. If Plasma ends the session during a run (the permission was revoked, or the screen locked), clicking stops, Plasma lets go of every button and key, and the tray says why.

The portal needs the package's `io.github.tobylo.AutoClicker.desktop`: Plasma keeps the permission per app. If the portal cannot be used, the program keeps running and the tray says why.

## Hotkeys

Hotkeys work in every window, fullscreen games included, because the program reads the keyboards directly. That needs read access to `/dev/input`: the user must be in the `input` group (see [Install](#install)). When no keyboard can be read, the program keeps running and the tray says why.

The keyboards are only read, never taken over, so the game also gets the hotkey. In Minecraft, Ctrl is sprint: pressing Ctrl+F6 also starts sprinting for a moment. Pick a hotkey the game ignores if that gets in the way.

If a key remapper (keyd, kanata) takes over a keyboard while Ctrl or another modifier is held on it, the program never sees that key come up, and hotkeys stay blocked until the keyboard is unplugged or the program restarts.

A run that holds a modifier itself (`hold_key` with `key = "leftshift"`, say) and a remapper that re-sends this program's virtual keyboard (keyd's default, which takes over every keyboard) do not mix: the hotkey then arrives with that modifier added, does not match, and cannot stop the run. Stop it from the tray, or set the remapper to leave the "auto-clicker virtual keyboard" device alone.
