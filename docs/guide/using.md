# Using it

Once installed and enabled, it starts at login and stays in the system tray (run `auto-clicker` to start it by hand). A left or right click on its icon opens the menu:

- **Start clicking** counts down `tray_start_delay` seconds (5 by default), so you can switch to the game, then clicks. The icon itself shows the seconds left, so you can see them with the game in front; a countdown over 99 seconds shows whole minutes, such as `5m`. **Stop clicking** stops it, or cancels the countdown.
- **Record** counts down the same way, then records what the mouse and keyboard do until **Stop recording**, and saves it. The icon turns red while recording. Where recording cannot start (with the hotkeys from Plasma's shortcuts, without the `input` group, or after watching for keyboards failed), Record is greyed out and its label says what it needs, such as `Record (needs the input group)`; the tooltip says how to fix it.
- **Play** lists the ten newest recordings, newest first. Choosing one counts down the same way, then plays it; **Stop playing** stops it, or cancels the countdown (see [Playing a recording](playback.md)).
- **Settings** opens the settings window (see [Settings window](settings-window.md)).
- **Quit** stops clicking and playback, lets go of everything it pressed, saves a recording in progress and exits.

The hotkey (Ctrl+Shift+F6 by default) starts and stops clicking at once, without a countdown. A start waits until you let go of Ctrl and Shift, so the first click is a plain click and not Ctrl+Shift+click; if they are still held after 30 seconds, nothing starts and the tooltip says why.

Clicking and playback stop when the screen locks, another user's session comes to the front, or the computer goes to sleep, and they cannot start again until the session is back. A recording stops and is saved at those moments too. Before sleep, the program holds the computer back (for 2 seconds at most) until it has let go of every button and key it pressed, so nothing stays pressed while asleep. The tooltip says what the clicker will do, and why it stopped or cannot start.

Only one copy runs per session: a second one prints "auto-clicker is already running" and exits. A running copy quits if it loses the session bus. It talks to the session and system buses through their Unix sockets only, and refuses to start when `DBUS_SESSION_BUS_ADDRESS` names anything else.

Exit statuses: 0 after Quit, SIGTERM or SIGINT; 3 when another copy is already running; 1 for any other failure at start. When neither `/dev/uinput` nor Plasma's remote input can be used, the program keeps running: the tooltip says why, and each start tries again.
