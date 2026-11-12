<img src="assets/icon.svg" width="64" height="64" alt="">

# auto-clicker

An auto clicker with macro record and playback for KDE Plasma on Wayland, living in the system tray. It is for anyone who plays games on a Plasma desktop and wants fast clicking, a held button or key, or a recorded routine, switched on and off by a hotkey that works inside fullscreen games. Written in Rust to stay light while idle.

Status: the auto clicker runs from the tray and the hotkey, records macros and plays them back, and installs as a package on CachyOS and Arch (see [Install](#install)). See [docs/planning/map.md](docs/planning/map.md) for what it will do and why, and [the Project](https://github.com/users/tobylo/projects/4) for the work.

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

1. To record macros, let them read the keyboards and mice (replace `NAME` with their login name). Without this, the hotkeys go through Plasma's own shortcuts instead and recording is unavailable (see [Choosing the route](#choosing-the-route)):
   `sudo usermod -aG input NAME`
2. Restart the computer, so the new group and the `/dev/uinput` rule take effect.
3. Logged in as that person, start it now and at every login:
   `systemctl --user enable --now auto-clicker`

The icon then appears in the system tray. After **Quit**, start it again from "Auto Clicker" in the app menu, or log in again.

### What the permissions mean

- **`/dev/uinput`** lets a program create a virtual mouse and keyboard, which is how auto-clicker clicks. The rule lets programs of the person logged in at the screen open it, any of their programs, not just this one. When someone else takes over the screen, no new program of the first person can open it, but one that already has it open keeps it. auto-clicker itself stops clicking and lets go of everything when the screen locks or another user's session comes to the front.
- **The `input` group** lets a program read every keyboard and mouse directly, which recording needs, and which lets the hotkeys work without Plasma's shortcuts. auto-clicker only looks for its hotkeys, and records keys only while you record a macro. But the group applies to every program that person runs, not just this one: only add people whose programs you trust.
- The background service may use local Unix sockets only, so the program cannot reach the network even by mistake. The program itself opens no network connection.

### Updating

The program does not update itself. To install a newer version, from the account that installed it:

1. Fetch the new commits and list them:
   `cd ~/auto-clicker && git fetch && git log --oneline HEAD..origin/main`
2. Read what they change, all of it in one diff:
   `git diff HEAD origin/main`
   The program presses keys and reads keyboards: don't go on past a change you don't trust.
3. Bring them in, then build and install:
   `cd ~/auto-clicker/packaging/arch && git merge --ff-only origin/main && makepkg -sif`

Then restart the computer, or, for each person, `systemctl --user restart auto-clicker`. Always use `-f`: without it, makepkg would reinstall the package it built last time.

The build refuses to run if the downloaded copy has been changed by hand (`git status` shows changes), so what gets installed is always a version from this repository.

### If it does not start

`systemctl --user status auto-clicker` shows whether it runs, and `journalctl --user -b -u auto-clicker` shows its messages. Exit status 3 means another copy is already running. When it runs but cannot click or read hotkeys, the tray's tooltip says why and what to do.

### Uninstall

1. For each person who enabled it, logged in as them:
   `systemctl --user disable --now auto-clicker`
2. Remove the package:
   `sudo pacman -Rns tobylo-auto-clicker`
3. Take each person out of the `input` group, unless something else needs it, then restart the computer: programs already running keep the group until then.
   `sudo gpasswd -d NAME input`
4. Optionally, from the account that installed it, delete the downloaded copy:
   `rm -rf ~/auto-clicker`
   and Rust, if the build installed it and nothing else of yours needs it:
   `sudo pacman -Rns rust`
5. Optionally, as each person, delete their settings:
   `rm -rf ~/.config/auto-clicker`

## Using it

Once installed and enabled, it starts at login and stays in the system tray (run `auto-clicker` to start it by hand). A left or right click on its icon opens the menu:

- **Start clicking** counts down `tray_start_delay` seconds (5 by default), so you can switch to the game, then clicks. The icon itself shows the seconds left, so you can see them with the game in front; a countdown over 99 seconds shows whole minutes, such as `5m`. **Stop clicking** stops it, or cancels the countdown.
- **Record** counts down the same way, then records what the mouse and keyboard do until **Stop recording**, and saves it. The icon turns red while recording. Where recording cannot start (with the hotkeys from Plasma's shortcuts, without the `input` group, or after watching for keyboards failed), Record is greyed out and its label says what it needs, such as `Record (needs the input group)`; the tooltip says how to fix it.
- **Play** lists the ten newest recordings, newest first. Choosing one counts down the same way, then plays it; **Stop playing** stops it, or cancels the countdown (see [Playing a recording](#playing-a-recording)).
- **Settings** opens the settings window (see [Settings window](#settings-window)).
- **Quit** stops clicking and playback, lets go of everything it pressed, saves a recording in progress and exits.

The hotkey (Ctrl+Shift+F6 by default) starts and stops clicking at once, without a countdown. A start waits until you let go of Ctrl and Shift, so the first click is a plain click and not Ctrl+Shift+click; if they are still held after 30 seconds, nothing starts and the tooltip says why.

Clicking and playback stop when the screen locks, another user's session comes to the front, or the computer goes to sleep, and they cannot start again until the session is back. A recording stops and is saved at those moments too. Before sleep, the program holds the computer back (for 2 seconds at most) until it has let go of every button and key it pressed, so nothing stays pressed while asleep. The tooltip says what the clicker will do, and why it stopped or cannot start.

Only one copy runs per session: a second one prints "auto-clicker is already running" and exits. A running copy quits if it loses the session bus. It talks to the session and system buses through their Unix sockets only, and refuses to start when `DBUS_SESSION_BUS_ADDRESS` names anything else.

Exit statuses: 0 after Quit, SIGTERM or SIGINT; 3 when another copy is already running; 1 for any other failure at start. When neither `/dev/uinput` nor Plasma's remote input can be used, the program keeps running: the tooltip says why, and each start tries again.

## Recording

A recording keeps the clicks, keys, wheel and mouse movement, with their timing, from the moment it starts until it stops. Start and stop it with Ctrl+Shift+F7, or from the tray. Keys record with any keyboard; clicks and movement need a mouse, since a laptop's touchpad is not recorded: its taps and gestures cannot be played back as mouse input. Keyboards and mice plugged in while recording are recorded too.

- **What is left out.** The hotkey that starts or stops the recording is not in it, nor is any other hotkey pressed while recording (they do nothing then). Ctrl held for a while before the stop, to sprint say, is kept. From the tray, the trip to the tray at the end (the moves and clicks to open the menu and choose Stop recording) is cut off. Key repeat is not recorded: a held key is one press and one release.
- **What is added.** Keys and buttons held when recording starts (W, to walk on) count as pressed at the start; those of the Ctrl+Shift+F7 that started it do not. Anything still held when the recording stops, or on a mouse or keyboard unplugged while recording, gets a release at that moment, so a recording always ends with nothing pressed.
- **Limits.** A recording stops and saves by itself after 60 minutes; the tooltip says so. A recording with nothing in it is not saved. Recording cannot start while clicking runs, and clicking cannot start while recording.
- **Where.** Each recording is a file in `~/.local/share/auto-clicker/recordings/` (or under `$XDG_DATA_HOME`), named after the time it was made, such as `2026-11-09_18-04-05.123.json`; it is named "Recording 2026-11-09 18:04" until you rename it in the settings window. The folder and files are readable by you only. A recording is written once, whole, when it stops: if the computer crashes while recording, that recording is lost, but no half file is left, and saving never replaces an older recording.
- **Privacy.** Keys are read for recording only while you record, and are written only to the recording. Nothing about them goes to the log or the tooltip.
- **Where the pointer was.** On Plasma, a recording also keeps where the pointer was on the screens, in whole pixels, and the screens it was made on, so playback can put a click on the same pixel. The program asks KWin for both through two small KWin scripts: one that reports the screens, loaded while the program runs, and one that reports the pointer, loaded only while you record. They need no permission and cost nothing while the pointer is still. They are written to a private folder (`$XDG_RUNTIME_DIR/auto-clicker/`) and the program unloads them when it quits; if KWin does not answer then, or the program crashed, the next start tries again. If the screens script fails to load or the pointer script fails to unload (KWin not answering), the program tries three more times, 2, 4 and 8 seconds apart; after that, it waits until KWin answers another call or restarts, the next recording, the program quitting, or its next start. While a game locks the pointer there is no position to keep, and the movement is recorded as before. Without KWin (another desktop), recordings keep movement only.

The files are JSON, one event per line: microseconds since the start, what happened, and the key, button or amount. Keys and buttons use the kernel's names (`KEY_W`, `BTN_LEFT`):

```json
{"version":1,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":2500000,"events":[
[0,"press","KEY_W"],
[1200,"move",3,-2],
[480000,"wheel",-1],
[2500000,"release","KEY_W"]
]}
```

A recording with positions is version 2: it adds the screens it was made on (each one's place and size in logical pixels, and its scale in thousandths) and `"at"` events with the pointer's position. Version 2 is written only when there are positions, so a recording without them stays version 1:

```json
{"version":2,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":52000,"layout":[{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250}],"events":[
[0,"at",640,400],
[1200,"move",3,-2],
[1900,"at",643,398],
[52000,"press","BTN_LEFT"],
[52000,"release","BTN_LEFT"]
]}
```

## Playing a recording

Playback repeats what a recording holds, with its timing: each key, click, wheel turn and mouse movement happens as long after the start as it did when it was recorded. Start it with Ctrl+Shift+F8, or from the tray's Play menu.

- **Which recording.** Ctrl+Shift+F8 plays the current recording: the one you last made, played or chose in the settings window. The program remembers it across restarts in `~/.local/state/auto-clicker/current-recording` (or under `$XDG_STATE_HOME`); until there is one, or when its file is gone, the newest plays. The tray lists the ten newest by the time in their file names, such as "Recording 2026-11-09 18:04:05", or "Farm loop (2026-11-09 18:04:05)" once renamed, and checks the current one. The tooltip names what is playing.
- **Starting.** From the tray, playback counts down `tray_start_delay` seconds first. From Ctrl+Shift+F8 it starts as soon as you let go of Ctrl and Shift, so they do not mix into what plays. If one is still held after 2 seconds it starts anyway, and the tooltip says so.
- **Stopping.** Any of the three hotkeys stops playback, and so does Stop playing in the tray. Whenever it stops, and however (a hotkey, the tray, a locked screen, sleep, Quit, an error), every key and button it was holding is let go at once.
- **Your own input.** The mouse and keyboard keep working while a recording plays and do not stop it: what you do mixes with what plays. A key you hold while the recording presses the same key stays down on Plasma until both have let go of it: the game sees one press and one release (measured on Plasma 6.8.2).
- **Speed and repeats.** `[playback]` in the [configuration](#configuration) sets the speed, from a quarter to four times the recorded one, and how many times the recording plays: a number, or until stopped. Each repeat starts a whole recording after the one before, the idle time at the recording's end included, so rounds do not run together; the last round is over with its last event. A recording shorter than a tenth of a second plays once only: repeated, it would spin faster than you could stop it.
- **Timing.** Every event is scheduled from the moment playback started, so small delays never add up, however long the recording or however many rounds. An event lands within a millisecond or two of its time (measured: 1.3 ms off at worst, recording the replay of a 20-event routine on the kernel's own clock). If the computer stalls, the events that became due play in order as soon as it can, and the ones after them keep their own times.
- **Mouse movement.** A recording with positions moves the pointer to each recorded position, so a click made with the pointer still lands on the pixel it was made on (within a pixel at scales up to about 1.8); a click made while the pointer moves fast can land a little along its path. Movement made while a game locked the pointer has no positions and plays back as the same mouse movement, which turns the view as recorded, except for a move within 5 ms before the game lets go of the pointer, which the position after it replaces. Positions are used when every screen they lie on is still where it was, with the same size; a screen added beside them changes nothing. Otherwise, or while the screens are not known (another desktop, or KWin restarting), the whole playback uses mouse movement and the tooltip says why. If the screens change while positions play, playback stops and says so.
- **Movement without positions.** Recordings without positions (made before positions existed, or away from Plasma) play back as mouse movement. On the desktop the pointer can then end somewhere else than it did: the desktop applies its pointer acceleration to the program's virtual mouse, and acceleration depends on how fast the movement is, so it also changes with the playback speed. Measured on Plasma 6.8.2 with its default acceleration, 400 units of movement moved the pointer 397 to 795 pixels, depending on how fast it was recorded and played (niri gave the same figures).
- **Refusals.** Playback does not start while clicking or recording, and those do not start while a recording plays. A file that is missing, damaged or not a recording is not played: the tooltip says what is wrong with it, and nothing is pressed.
- **Files.** Only files named the way the program names them appear in the Play menu. The program deletes a recording only when you delete it in the settings window; a long one with much mouse movement can take tens of megabytes, so clear out old ones now and then.

## Settings window

**Settings** in the tray's menu opens a window where you change every setting below without editing the file. It is a small program of its own, `auto-clicker-settings`, which the daemon starts and which exits when you close it, so the daemon stays as light as before while you play. Choosing Settings again while the window is open does not open a second one; Wayland does not let the program bring the open window forward, so click it in the task manager.

- **Clicker** sets what a run does: the action, the interval (milliseconds between actions, or actions a second), jitter, the button, the key, where to click and how often. The key is picked by pressing it: choose **Change…** and press the key, or Esc to cancel. When Plasma delivers the hotkeys, the program reads no keyboard, so there is no **Change…**: type the key's name, such as `space`, or its code after `#`, such as `#113`. The fixed point is picked on the screen: choose **Pick on the screen…**, and every screen dims under an overlay; click where the clicker should click, or press Esc to cancel (the overlay also closes by itself after 30 seconds). The window fills in the point, and **Save** keeps it. Picking needs the screens from KWin and works only while nothing is clicking, recording or playing. A picked point is clicked where you clicked to within 1/131072 of the width or height of all your screens together: 0.04 logical px across a laptop panel and a 3440 px monitor side by side, and always on the screen you picked it on. It is saved with the screens it was picked on, so clicking at it starts only on those screens and stops if they change: after you dock or undock, pick it again. The point can also be typed as two numbers, which keeps no screens.
- **Hotkeys** shows the three hotkeys. To change one, choose **Change…** and press the new combination, such as Ctrl+Alt+F9, or Esc to cancel; the program's hotkeys do nothing while it waits, for up to 10 seconds. A hotkey without Ctrl, Shift, Alt or Meta fires on a plain key press, in games too, so the window asks before saving one. When the hotkeys come from Plasma's shortcuts (`hotkeys = "portal"`, or `"auto"` that picked them at start), Plasma owns the hotkeys: the window shows the ones Plasma reports, and **Change in System Settings** opens Plasma's shortcut settings for the program. This tab also sets where hotkeys come from and where clicks go (**Picked at start** is `auto`), which apply after the program restarts.
- **Playback** sets the speed and how many times a recording plays, which apply from the next playback once saved. Below them is the recordings list, newest first, with a ✓ on the one Ctrl+Shift+F8 plays; it shows the newest 200 and says how many older ones there are. Pick a recording to **Rename** it (the name inside its file changes, the file name stays; two recordings may share a name, and the time tells them apart), **Play** it after the tray countdown, make it the one the hotkey plays (**Play with the hotkey**), or **Delete…** it for good after a Yes/No. These apply at once, without Save, and wait like a save until nothing is clicking, recording or playing. A rename that would make the file larger than a recording may be is refused, and when a change is made but may not survive a power loss, the window says so. The tray's Play menu follows every change.

**Save** checks the settings as the program checks the file, says what is wrong if something is, and writes the file. Saving waits until nothing is clicking, recording or playing. Saved hotkeys work at once; the other settings apply to the next start. If the file changed since the program read it, or could not be used, the window asks before replacing it. The window speaks Swedish when the system language is Swedish, and so do the tray's menu, title and tooltip, with every notice and problem; text quoted from another program, such as an error from the system, stays as that program wrote it. The window follows Plasma's light or dark colours.

## Configuration

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
- `[input]` chooses the route clicks and hotkeys take; see [Choosing the route](#choosing-the-route).

- A hotkey is any of Ctrl, Shift, Alt and Meta, then a key name from the list above, joined by `+`, in any letter case: "Ctrl+F6", "ctrl+shift+f9", "F13". It fires when the key is pressed while exactly those modifiers are held, left or right alike, on any keyboard. Two actions cannot share a hotkey, and with `hotkeys = "kernel"` or `"auto"` no hotkey can use the key the clicker presses (with `hotkeys = "portal"`, Plasma's own hotkeys decide that).

### Choosing the route

Clicks and hotkeys each take one of two routes: the kernel's (`/dev/uinput` for clicks, the keyboards in `/dev/input` for hotkeys) or Plasma's portals (remote input for clicks, its own shortcuts for hotkeys). The program picks each at start, and the tooltip names the two it uses, such as "Clicks through /dev/uinput, hotkeys from the desktop's shortcuts".

- `auto`, the default, takes the kernel's route where the program has access and the portal otherwise. For clicks, that is `/dev/uinput`, which the package's rule opens to the person at the screen; Plasma's remote input only where `/dev/uinput` cannot be opened. For hotkeys, the keyboards when one with the hotkey keys can be read (the `input` group gives that); Plasma's shortcuts when the keyboards may not be read. A gamepad or other device that can be read does not count: only a keyboard does. With no keyboard plugged in at start, the program waits for one.
- `kernel` or `portal` uses that route whatever the access. Without access, the program keeps running, and the tooltip says why and what to do.
- The two settings are independent. After installing as above without the `input` group, `auto` clicks through `/dev/uinput` with no prompt and takes the hotkeys from Plasma's shortcuts.
- The route is chosen once, at start. After changing `[input]`, restart the program. A new `input` group membership reaches programs only after a new login: log out and in again, or restart the computer.

| Feature | Set by | `kernel` | `portal` |
| --- | --- | --- | --- |
| Clicking and playback | `output` | Through `/dev/uinput`, with no prompt | Through Plasma's remote input; Plasma asks once |
| Hotkeys | `hotkeys` | Read from the keyboards, in every window; needs the `input` group; set in `[hotkeys]` | Plasma's own shortcuts, in every window; no `input` group; set in System Settings |
| Recording | `hotkeys` | Yes; needs the `input` group | No: nothing reads the keyboards and mice, so Record is greyed out and reads `Record (needs the input group)` |

## Clicking through the portal

With `output = "portal"`, or `"auto"` where `/dev/uinput` cannot be opened, the program asks Plasma for remote input (the RemoteDesktop portal) instead of creating virtual devices, so it needs no access to `/dev/uinput`. The first time you start clicking, Plasma asks whether "Auto Clicker" may control the keyboard and pointer. After you allow it, clicking does not begin by itself, since the dialog took the focus from your game: switch back and start again. Plasma remembers the answer, so later starts go ahead without asking; the program keeps Plasma's token for it in `~/.local/state/auto-clicker/portal-token`. If you deny it, the tray says so and the next start asks again. Plasma lists the permission under System Settings, where it can be revoked.

The program opens a remote input session for each run and closes it when the run stops, so Plasma shows that remote control is active only while clicking. If Plasma ends the session during a run (the permission was revoked, or the screen locked), clicking stops, Plasma lets go of every button and key, and the tray says why. Playing a recording goes the same way: a session for each playback, closed when it ends.

The portal needs the package's `io.github.tobylo.AutoClicker.desktop`: Plasma keeps the permission per app. If the portal cannot be used, the program keeps running and the tray says why.

## Hotkeys

With `hotkeys = "kernel"`, or `"auto"` where a keyboard can be read, hotkeys work in every window, fullscreen games included, because the program reads the keyboards directly. Mice are read only while recording, so the program sleeps while you play. That needs read access to `/dev/input`: the user must be in the `input` group (see [Install](#install)). When no keyboard can be read, the program keeps running and the tray says why; without the `input` group, Record is greyed out too.

The keyboards are only read, never taken over, so the game also gets the hotkey. The defaults are free in stock Plasma, and Minecraft does nothing on F6, F7 or F8; a hotkey Plasma also uses (Ctrl+F7 is its "Toggle Present Windows (Window class)") would do both things at once. In Minecraft, Ctrl is sprint and Shift is sneak: pressing Ctrl+Shift+F6 also does both for a moment. Pick a hotkey the game ignores if that gets in the way.

If a key remapper (keyd, kanata) takes over a keyboard while Ctrl or another modifier is held on it, the program never sees that key come up, and hotkeys stay blocked until the keyboard is unplugged or the program restarts.

A run that holds a modifier itself (`hold_key` with `key = "leftshift"`, say) and a remapper that re-sends this program's virtual keyboard (keyd's default, which takes over every keyboard) do not mix: the hotkey then arrives with that modifier added, does not match, and cannot stop the run. Stop it from the tray, or set the remapper to leave the "auto-clicker virtual keyboard" device alone.

## Contributing

Bug reports and ideas are welcome as [issues](https://github.com/tobylo/auto-clicker/issues). The program installs only from this repository, and a pull request from outside is merged only by the maintainer, so it may wait a while or be turned down. Report a security problem privately, with "Report a vulnerability" under the repository's [Security tab](https://github.com/tobylo/auto-clicker/security), not in a public issue.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit for inclusion is licensed the same way, with no additional terms.

## Hotkeys through the portal

With `hotkeys = "portal"`, or `"auto"` where no keyboard may be read, the program reads no keyboard: it registers its three shortcuts with Plasma (the GlobalShortcuts portal), and Plasma tells it when one is pressed, in every window and in fullscreen games. The first time the program starts this way, Plasma shows a dialog listing the shortcuts; accept it. The program asks for the hotkeys in `[hotkeys]` then, for those on a function key (F1 to F24); any other starts without a key, for you to set in System Settings.

From then on Plasma owns the hotkeys, and `[hotkeys]` in the config has no further effect on this route. Change them in System Settings, under Keyboard, Shortcuts, "Auto Clicker". A change takes effect at once, without a restart, and the tray's tooltip shows the hotkeys as Plasma reports them.

- **A hotkey Plasma already uses never fires.** Plasma still lists it as set (Ctrl+F7 is its own "Toggle Present Windows (Window class)", for one). If a hotkey shown in the tooltip does nothing, pick another in System Settings.
- **A start from the hotkey begins 1 second after you let go.** Plasma reports the release when the first key comes up, usually the function key while Ctrl and Shift are still down, and never says when those follow. The pause keeps the first clicks from being Ctrl+Shift+clicks. Playback still starts 2 seconds after the press at the latest, as on the other route, so let go of its hotkey within a second.
- **The clicker's key cannot be a hotkey's key.** Plasma cannot tell the program's key presses from yours, so `key_repeat` and `hold_key` do not start when their key is the key of any hotkey, whatever its modifiers, and the tooltip says so. They do not start either when the key is Ctrl, Shift, Alt or Meta (held by the program, it would turn your Ctrl+Shift+F6 into something else), or when a hotkey is not on a function key (F1 to F24): Plasma names other keys by the keyboard layout's labels, which the program cannot match against its own. Click modes always start.
- **A recording that presses a hotkey's key is not played,** for the same reason: played back, it would set the hotkey off, and any hotkey stops playback. Nor is a recording with keys played while a hotkey is not on a function key. Recordings that hold Ctrl, Shift, Alt or Meta play; while one of those is held by the playback, your hotkey may not match, so stop from the tray then.
- **Recording is not available.** Nothing reads the keyboards and mice on this route, so the tray's Record item is greyed out and reads `Record (needs the input group)`, and the tooltip says `Recording is unavailable with the desktop's shortcuts: nothing reads the keyboards and mice. It needs the input group, and hotkeys = "auto" or "kernel" under [input] in the config.` The record shortcut stays in System Settings; pressing it records nothing, and the tooltip says why.
- **If one of Plasma's portal services stops or restarts,** the program registers its shortcuts again at once, without a dialog. For one second after that, hotkeys do nothing: a key the program itself pressed meanwhile may have set one off. The same second follows when you move a hotkey onto a key a run is pressing, which stops the run. If that fails, the tooltip says hotkeys are unavailable, clicking goes on working from the tray, and the program tries again when a portal service next starts or stops. After five tries in a row that fail, it stops trying and the tooltip says to restart it. While hotkeys are unavailable, nothing can set one off, so the key modes start whatever their key is.
- **If you decline the dialog,** hotkeys stay unavailable. Plasma does not ask a second time: restart the program and set the hotkeys in System Settings.

Like remote input, this needs the package's `io.github.tobylo.AutoClicker.desktop`: Plasma keeps the shortcuts per app.
