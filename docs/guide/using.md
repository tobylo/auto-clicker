# Using it

Once installed and enabled, it starts at login and stays in the system tray (run `auto-clicker` to start it by hand). A left or right click on its icon opens the menu:

- **Start clicking** counts down `tray_start_delay` seconds (5 by default), so you can switch to the game, then clicks. The seconds left show in the middle of every screen, above the game, and on the icon; a countdown over 99 seconds shows whole minutes, such as `5m` (see [The countdown on the screen](#the-countdown-on-the-screen)). **Stop clicking** stops it, or cancels the countdown.
- **Record** counts down the same way, then records what the mouse and keyboard do until **Stop recording**, and saves it. The icon turns red while recording. Where recording cannot start (with the hotkeys from Plasma's shortcuts, without the `input` group, or after watching for keyboards failed), Record is greyed out and its label says what it needs, such as `Record (needs the input group)`; the tooltip says how to fix it.
- **Play** lists the ten newest recordings, newest first. Choosing one counts down the same way, then plays it; **Stop playing** stops it, or cancels the countdown (see [Playing a recording](playback.md)).
- **Settings** opens the settings window (see [Settings window](settings-window.md)).
- **Quit** stops clicking and playback, lets go of everything it pressed, saves a recording in progress and exits.

The hotkey (Ctrl+Shift+F6 by default) starts and stops clicking at once, without a countdown. A start waits until you let go of Ctrl and Shift, so the first click is a plain click and not Ctrl+Shift+click; if they are still held after 30 seconds, nothing starts and the tooltip says why.

## The countdown on the screen

During a countdown from the tray, the seconds left show as large digits in the middle of every screen, above everything else, a fullscreen game included: indigo before clicking or playback, red before a recording, with a thin white outline so they read on light and dark scenes. Nothing else is drawn, not even a backdrop. The number goes as soon as the run begins or the countdown is cancelled. It takes no keyboard and no clicks, so the game keeps its focus, its clicks and its locked pointer. A start from a hotkey has no countdown, so nothing shows. `countdown = false` under `[on_screen]` turns it off (see [Configuration](configuration.md)); the icon keeps counting down either way.

It needs a desktop that offers layer-shell: Plasma, niri and Hyprland do, GNOME does not. Where it cannot show, the tooltip says why, such as "The countdown cannot show on the screen: this desktop cannot show it (no layer-shell)". If the desktop closes the number on one screen or does not answer in time, that screen shows nothing until the next countdown or until a screen is added or removed; the others keep theirs.

Screenshots, recordings and streams of the screen include the number. Its surfaces carry the namespace `auto-clicker-on-screen`, so a compositor rule can leave them out:

- **niri:** in `~/.config/niri/config.kdl`:
  ```kdl
  layer-rule {
      match namespace="^auto-clicker-on-screen$"
      block-out-from "screencast"
  }
  ```
- **Hyprland:** in `~/.config/hypr/hyprland.lua`:
  ```lua
  hl.layer_rule({ match = { namespace = "^auto-clicker-on-screen$" }, no_screen_share = true })
  ```
- **Plasma:** KWin has no such rule; turn the countdown off with `countdown = false` before streaming.

Clicking and playback stop when the screen locks, another user's session comes to the front, or the computer goes to sleep, and they cannot start again until the session is back. A recording stops and is saved at those moments too. Before sleep, the program holds the computer back (for 2 seconds at most) until it has let go of every button and key it pressed, so nothing stays pressed while asleep. The tooltip says what the clicker will do, and why it stopped or cannot start.

Only one copy runs per session: a second one prints "auto-clicker is already running" and exits. A running copy quits if it loses the session bus. It talks to the session and system buses through their Unix sockets only, and refuses to start when `DBUS_SESSION_BUS_ADDRESS` names anything else.

Exit statuses: 0 after Quit, SIGTERM or SIGINT; 3 when another copy is already running; 1 for any other failure at start. When neither `/dev/uinput` nor Plasma's remote input can be used, the program keeps running: the tooltip says why, and each start tries again.
