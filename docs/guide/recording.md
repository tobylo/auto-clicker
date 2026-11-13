# Recording

A recording keeps the clicks, keys, wheel and mouse movement, with their timing, from the moment it starts until it stops. Start and stop it with the `toggle_recording` hotkey (Ctrl+Shift+F7 by default), or from the tray. Keys record with any keyboard; clicks and movement need a mouse, since a laptop's touchpad is not recorded: its taps and gestures cannot be played back as mouse input. Keyboards and mice plugged in while recording are recorded too.

- **What is left out.** The hotkey that starts or stops the recording is not in it, nor is any other hotkey pressed while recording (they do nothing then). Ctrl held for a while before the stop, to sprint say, is kept. From the tray, the trip to the tray at the end (the moves and clicks to open the menu and choose Stop recording) is cut off. Key repeat is not recorded: a held key is one press and one release.
- **What is added.** Keys and buttons held when recording starts (W, to walk on) count as pressed at the start; those of the hotkey that started it do not. Anything still held when the recording stops, or on a mouse or keyboard unplugged while recording, gets a release at that moment, so a recording always ends with nothing pressed.
- **Limits.** A recording stops and saves by itself after 60 minutes, or earlier if its file would grow past 256 MiB (a lot of fast mouse movement); the tooltip says so. A recording with nothing in it is not saved. Recording cannot start while clicking runs, and clicking cannot start while recording.
- **Where.** Each recording is a file in `~/.local/share/auto-clicker/recordings/` (or under `$XDG_DATA_HOME`), named after the time it was made, such as `2026-11-09_18-04-05.123.json`; it is named "Recording 2026-11-09 18:04" until you rename it in the settings window. The folder and files are readable by you only. A recording is written once, whole, when it stops: if the computer crashes while recording, that recording is lost, but no half file is left, and saving never replaces an older recording.
- **Privacy.** Keys are read for recording only while you record, and are written only to the recording. Nothing about them goes to the log or the tooltip.
- **Where the pointer was.** On Plasma and Hyprland, a recording also keeps where the pointer was on the screens, in whole pixels, and the screens it was made on, so playback can put a click on the same pixel. The screens come from the Wayland display (xdg-output), as on any desktop. The pointer comes from a small KWin script, loaded only while you record, which also reports the screens as KWin sees them: a position is kept only while those are the screens the recording was made on. The script needs no permission and costs nothing while the pointer is still. It is written to a private folder (`$XDG_RUNTIME_DIR/auto-clicker/`) and the program unloads it when the recording ends and when it quits; if KWin does not answer then, or the program crashed, the next start tries again (it also unloads the screens script that earlier versions kept loaded). If the script fails to unload (KWin not answering), the program tries three more times, 2, 4 and 8 seconds apart; after that, it waits until KWin answers another call or restarts, the next recording, the program quitting, or its next start. When KWin restarts, the program reads the screens from the display again once the new KWin is up. While a game locks the pointer there is no position to keep, and the movement is recorded as before. On Hyprland the pointer comes from Hyprland's own socket instead, asked only while you record: once when the recording starts, then at most once a millisecond while the mouse moves, and nothing while it is still. Its answers are in whole pixels, rounded down, and an answer is kept only when the screens did not change around it. The program uses the socket `HYPRLAND_INSTANCE_SIGNATURE` names only when it belongs to the same Hyprland as the Wayland display, so a signature left over from another session is never asked. If Hyprland does not answer within 50 ms (it holds its answers while a screen changes mode), that moment keeps movement only and the tooltip says "Hyprland's socket did not answer" until an answer comes or the recording stops. Where nothing reports the pointer (niri, or Hyprland without its signature), or while the screens are not known (no Wayland display, a desktop without xdg-output, or the moment KWin takes to restart), recordings keep movement only: the Record item reads `Record (movement only)` and the tooltip says why, such as "Recordings keep movement only: nothing on this desktop reports where the pointer is". A recording begun while the screens are not known keeps movement only to its end, even once they are known again, and the tooltip says so until it stops. A recording keeps no positions either while its screens differ from those it began on, or, on Hyprland, while the display does not say what they are; the tooltip does not say so then. While KWin's script fails to load, the tooltip says that too, until a load works.

The files are JSON, one event per line: microseconds since the start, what happened, and the key, button or amount. Keys and buttons use the kernel's names (`KEY_W`, `BTN_LEFT`):

```json
{"version":1,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":2500000,"events":[
[0,"press","KEY_W"],
[1200,"move",3,-2],
[480000,"wheel",-1],
[2500000,"release","KEY_W"]
]}
```

A recording with positions is version 2 on Plasma: it adds the screens it was made on (each one's place and size in logical pixels, and its scale in thousandths) and `"at"` events with the pointer's position. Version 2 is written only when there are positions, so a recording without them stays version 1:

```json
{"version":2,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":52000,"layout":[{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250}],"events":[
[0,"at",640,400],
[1200,"move",3,-2],
[1900,"at",643,398],
[52000,"press","BTN_LEFT"],
[52000,"release","BTN_LEFT"]
]}
```

A recording whose positions came from Hyprland is version 3: version 2 with `"positions_from":"hyprland"` after `duration_us`. Its positions were rounded down, so playback puts each one in the middle of its pixel, and it plays every mouse movement it holds, wherever it plays (see [Playing a recording](playback.md)). Builds older than this one do not read version 3.
