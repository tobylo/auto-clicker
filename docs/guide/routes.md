# Routes: kernel or portal

## Choosing the route

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

With `output = "portal"`, or `"auto"` where `/dev/uinput` cannot be opened, the program asks Plasma for remote input (the RemoteDesktop portal) instead of creating virtual devices, so it needs no access to `/dev/uinput`. The first time you start clicking, Plasma asks whether "Auto Clicker" may control the keyboard and pointer. After you allow it, clicking does not begin by itself, since the dialog took the focus from your game: switch back and start again. Plasma remembers the answer, so later starts go ahead without asking; the program keeps Plasma's token for it in `~/.local/state/auto-clicker/portal-token` (or under `$XDG_STATE_HOME`). If you deny it, the tray says so and the next start asks again. Plasma lists the permission under System Settings, where it can be revoked.

The program opens a remote input session for each run and closes it when the run stops, so Plasma shows that remote control is active only while clicking. If Plasma ends the session during a run (the permission was revoked, or the screen locked), clicking stops, Plasma lets go of every button and key, and the tray says why. Playing a recording goes the same way: a session for each playback, closed when it ends.

The portal needs the package's `io.github.tobylo.AutoClicker.desktop`: Plasma keeps the permission per app. If the portal cannot be used, the program keeps running and the tray says why.

## Hotkeys through the keyboards

With `hotkeys = "kernel"`, or `"auto"` where a keyboard can be read, hotkeys work in every window, fullscreen games included, because the program reads the keyboards directly. Mice are read only while recording, so the program sleeps while you play. That needs read access to `/dev/input`: the user must be in the `input` group (see [Install](install.md#for-each-person-who-uses-it)). When no keyboard can be read, the program keeps running and the tray says why; without the `input` group, Record is greyed out too.

The keyboards are only read, never taken over, so the game also gets the hotkey. The defaults are free in stock Plasma, and Minecraft does nothing on F6, F7 or F8; a hotkey Plasma also uses (Ctrl+F7 is its "Toggle Present Windows (Window class)") would do both things at once. In Minecraft, Ctrl is sprint and Shift is sneak: pressing Ctrl+Shift+F6 also does both for a moment. Pick a hotkey the game ignores if that gets in the way.

If a key remapper (keyd, kanata) takes over a keyboard while Ctrl or another modifier is held on it, the program never sees that key come up, and hotkeys stay blocked until the keyboard is unplugged or the program restarts.

A run that holds a modifier itself (`hold_key` with `key = "leftshift"`, say) and a remapper that re-sends this program's virtual keyboard (keyd's default, which takes over every keyboard) do not mix: the hotkey then arrives with that modifier added, does not match, and cannot stop the run. Stop it from the tray, or set the remapper to leave the "auto-clicker virtual keyboard" device alone.

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
