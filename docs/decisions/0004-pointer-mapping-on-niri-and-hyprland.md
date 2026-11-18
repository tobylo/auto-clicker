# 0004: The screen layout and the pointer on niri and Hyprland

- Status: accepted, 2026-11-12; Hyprland measured by W01c, 2026-11-13; KWin's absolute moves under a lock by W01d, 2026-11-13; niri's cursor sessions by W02, 2026-11-17; niri's absolute moves under a lock, and what they do to playback, by W04, 2026-11-18. niri and Plasma measured. W01a added niri at a negative position (one output); two outputs wait for a second monitor.
- Issue: W01 (#75). It shapes the slices in the [W01 brief](../planning/features/w01-other-desktops.md).
- Measured on: the dev box, niri 26.04 (8ed0da4), and for W02 niri-git 689c1e9 on Linux 7.3.5-1-cachyos with a second output (DP-4); Linux 7.3.3-1-cachyos, one output (HDMI-A-1, LG 34GN850, 3440x1440 at 85 Hz); the laptop, KWin 6.8.2, eDP-1 (2560x1600) and DP-1 (3440x1440). Hyprland 0.57.2 on both machines (below). Read from source: niri v26.04 (commit 8ed0da4; 6a0a862 is its tag object) with its pinned smithay (ff5fa7d), Hyprland v0.57.2 (11cbc6c; main at 8333bb7 for the pointer lock), KWin v6.8.2.

## Decision

- **niri maps the absolute device over the bounding box of all outputs,** as KWin does, so decision 0001's formula holds there, with W and H the box's size. At a fractional scale where physical/scale is not a whole number, niri's box (ceil) can be one logical px larger than the size xdg-output reports (round), so a point lands up to 1 logical px off near the far edges. Accepted (W01 decision 8): no workaround.
- **xdg-output is the layout source on every desktop.** niri and Hyprland report each output's logical position and size there; on Plasma it matched KWin's own layout in every layout tried, so Plasma moves to it too (W01b). The daemon moves the bounding box to (0, 0).
- **niri gives no pointer position in 26.04.** Recordings there keep relative movement. On niri's main branch, ext-image-copy-capture cursor sessions give it, and W02 records from them ([niri cursor sessions](#niri-cursor-sessions-w02)): positions mapped from physical to the nearest whole logical px, played as KWin's (version 2).
- **Hyprland maps the absolute device over the bounding box of all outputs** (measured on one and two outputs, negative positions included, at 1, 1.25 and 1.6; it replaces 1.5 by 1.6 where 1.5 leaves a fractional size). xdg-output matches `hyprctl monitors`, so decision 0001's formula holds on the box moved to (0, 0), within 0.002 logical px.
- **Hyprland's positions come from `cursorpos` (Route A),** asked on each mouse move at most once per ms plus one last ask: no stale answer in the runs measured (about 25,000 answers), inside the 5 ms cover window as KWin's script is, somewhat more often over the 2 ms delivery delay than KWin's script (2.2 % of moves against 0.29 %), and no frame lost in a game. Cursor sessions (Route B) miss the 2 ms window.
- **A recording made on Hyprland sends every relative move at playback, covered or not, then the position** (W01 decision 15; decision 17 ties the rule to the recording, version 3, wherever it plays, and keeps the skip rule for version 2 on any desktop). Hyprland's locked pointer drifts (by a few px in most runs), so `cursorpos` changes while a game holds the pointer and would cover the game's moves; nothing tells a lock from outside. Hyprland ignores absolute moves while a lock holds, so a locked game gets all its moves and no position; on the desktop the position follows each move within 5 ms. This changes R03 decision 8's skip rule for Hyprland's recordings only. KWin also ignores absolute moves while a window holds the pointer (measured under W01d, below), so such a recording keeps a game's moves on Plasma too.
- **W4's bound on Hyprland is 10 ms,** for a position stamped when its answer is read: the layout's publish trailed a position on the new screens by 3.12 ms at most.
- **Playback on niri keeps its rule; niri is asked to fix absolute moves under a lock (W04, 2026-11-18).** niri moves a locked pointer on an absolute move and sends the locked window motion, against pointer-constraints, so a recording's position off a game's surface takes the pointer out of the game; a recording made inside the game plays as on Plasma while its position is still on the game's surface. Nothing outside niri tells a client of a lock, and telling it from missing positions would need cursor sessions during playback, every move sent, and would still misread a screen's edge. So no playback rule: the owner reports it upstream with the facts below ("For an upstream report to niri"). A rule would be reconsidered if niri declines the fix.

## niri measurements

A virtual mouse shaped like C01's (buttons, `REL_X`, `REL_Y`, `REL_WHEEL`, `ABS_X` and `ABS_Y` over 0..=65535; python-evdev) moved to an 8x8 grid of absolute values (0, 1, 1000, 16384, 32768, 49152, 64000 and 65535 per axis). A fullscreen GTK 4 window (spikes/x01-cursor-position/harness/probe.py) printed where it saw the pointer. Predicted is `v * W / 65536` with W and H the size niri reports. Each scale was set for the run with `niri msg output HDMI-A-1 scale <s>` and set back to 1 after.

| Scale | Reported (IPC) | xdg-output | Box the device maps over | Worst error, logical px |
| --- | --- | --- | --- | --- |
| 1 | 3440x1440 | 3440x1440 | 3440x1440 | 0.002 |
| 1.25 | 2752x1152 | 2752x1152 | 2752x1152 | 0.003 |
| 1.4 | 2457x1028 | 2457x1029 | 2458x1029 | 0.977 |
| 1.5 | 2293x960 | 2293x960 | 2294x960 | 0.977 |

- At 1.4, `v = 32768` landed at x = 1229.00 and y = 514.50: half of 2458 and of 1029. At 1.5 it landed at 1147.00: half of 2294.
- `v = 65535` landed past the reported width at 1.4 and 1.5, outside the probe window, so the probe saw nothing there.
- The three sizes come from three roundings of physical/scale. smithay's `Space::output_geometry` rounds up (`to_i32_ceil`, `src/desktop/space/mod.rs:398`), and niri's pointer box is the union of those (`src/input/mod.rs:274-284`, used at 2646-2649). smithay's xdg-output rounds to nearest (`src/wayland/output/xdg.rs:67-71`). niri's IPC truncates (`src/utils/mod.rs:208-228`, `size.w as u32`).
- niri maps every libinput pointer over the whole box: `NiriInputDevice::output` returns `None` for a libinput device (`src/input/backend_ext.rs:27`). Two outputs were not measured: the dev box has one monitor. An output at a negative position was (below).

### One output at a negative position (W01a, 2026-11-12)

The same device and probe, with `niri msg output HDMI-A-1 position set -- -500 -300` at scale 1 (set back to 0, 0 after), over a 7x7 grid (0, 1, 1000, 16384, 32768, 49152 and 64000 per axis); the script is `.claude/local/w01a/niri_negative.py` in the main checkout. xdg-output reported the output at (-500, -300), 3440x1440; the daemon's layout source published it moved to (0, 0), 3440x1440, scale 1000 (`the_running_compositor_gives_a_layout_and_its_listener`). Every point landed at `v * W / 65536` from the output's own corner, worst error 0.002 logical px: niri maps over the box wherever it lies, so moving the box to (0, 0) keeps decision 0001's formula. Two outputs, one at a negative position, remain to measure.
- It matters where a scale's logical size is not whole: 3440 px at 1.4 or 1.5, 2560 px at 1.5. Common pairs such as 2560x1600 at 1.25 or 3840x2160 at 1.5 are exact. Today it affects only the picked fixed point, since niri gives no positions.

### For an upstream report (the owner files it)

- **Reproducer:** on one output of 3440x1440 at scale 1.5, a uinput device with `ABS_X` over 0..=65535 sends `ABS_X = 32768`. The pointer lands at x = 1147.0 logical, half of 2294. xdg-output and `niri msg outputs` both say the output is 2293 wide, so a client expects 1146.5.
- **Expected:** absolute motion is mapped over the same logical size the compositor reports to clients.
- **Where:** smithay `src/desktop/space/mod.rs:398` (`to_i32_ceil`) against `src/wayland/output/xdg.rs:67-71` (`to_i32_round`); niri `src/input/mod.rs:274-284` and 2646-2649.
- **Effect:** a tool that maps a logical point to an absolute device value lands up to 1 logical px (1.5 physical px at scale 1.5) off near the right and bottom edges.
- **Already reported?** No match on 2026-11-12 in niri's or smithay's issues for absolute pointer, fractional scale, `to_i32_ceil`, or xdg-output logical size rounding.

## niri and the pointer position

- niri 26.04's IPC has no cursor request (`niri-ipc` `Request`: Version, Outputs, Workspaces, Windows, Layers, KeyboardLayouts, FocusedOutput, FocusedWindow, PickWindow, PickColor, Action, Output, EventStream, ReturnError, OverviewState, Casts).
- Upstream, PR #1924 (`niri msg pointer`) was closed in 2025-08 after the maintainer questioned its uses. PR #4348 (`cursor-state`) has been open since 2026-07. On 2026-11-07 the maintainer pointed to ext-image-copy-capture-v1 cursor sessions as the way to follow the pointer without capturing frames.
- Those sessions are on niri's main branch (`src/handlers/image_copy_capture.rs`, reworked 2026-11-08), not in 26.04, the latest release (2026-04-25). On main they are refreshed after each input dispatch (`Niri::refresh_image_copy_cursor_sessions`), in physical px rounded to whole.
- niri 26.04's globals include `zxdg_output_manager_v1` (v3), `zwlr_output_manager_v1` and `zwlr_screencopy_manager_v1`, but no ext-image-copy-capture.

## niri cursor sessions (W02)

Read from source at niri 5935130 with its pinned smithay (65aa86f); measured on the dev box with niri-git 689c1e9, whose 15 later commits touch none of the code below (three change `src/input/mod.rs` elsewhere, so its lines below are 2811-2858 and 3032-3100 at 689c1e9).

- **What a session sends.** smithay sends a session's position only when it changed (`src/wayland/image_copy_capture/mod.rs:482-505`): `enter`, `hotspot` and `position` when the cursor comes onto the output, `position` on each change, `leave` when it goes. niri computes it as round((pointer − output's logical corner) × scale), in physical px in the output's displayed orientation (`src/niri.rs:6340`), and refreshes it after each pass of its event loop (`refresh_and_flush_clients`), not per frame. A session reports while any part of the cursor's image overlaps its output, so a position can be negative or past the output's size; that one is the neighbour's. The first position is sent as the session is made, with no capture session; a session niri cannot give constraints is stopped with no event (`mod.rs:1054-1102`), and an output that goes drops its sessions with no event.
- **No screen cast.** niri counts a cursor session as a cast only from its second frame (`src/handlers/image_copy_capture.rs:76-80`, `src/ipc/server.rs:935`); the program never asks for a frame.
- **A game holding the pointer.** `pointer_visibility` is niri's own hiding (hide when typing, after a while, on touch), not a lock. Under an active locked pointer, relative motion returns before the pointer moves (`src/input/mod.rs:2822-2839`): the pointer does not drift, so no position comes and KWin's skip rule holds (R03 decision 8). niri's absolute motion has no lock check (`src/input/mod.rs:3014-3088`): unlike KWin and Hyprland, niri moves a locked game's pointer on an absolute move, which matters for a Hyprland recording (version 3) played on niri.
- **The scale.** niri's scales are multiples of 1/120 (`src/utils/scale.rs:53-58`); a client sees no fractional scale, only the mode and xdg-output's logical size, round(mode / scale) (smithay `src/wayland/output/xdg.rs:70`). The program takes the one step that gives back both logical sides, else mode over logical size (at most 0.5 logical px off across the output). The layout's stored thousandths would be up to 3 physical px off at 7/6.
- **The error,** worst per axis in physical px over a 3440 x 1440 output, stored as whole logical px and played through the absolute device (65536 steps): 0.52 at scale 1, 1.02 at 1.25 and 1.03 at 1.5, with 99 % within 0.50, 0.98 and 0.99 (computed; fractions of a px stored would give 0.52 everywhere). The 0.02 to 0.03 past 1 is the device's quantisation, which every source shares. On top comes niri's ceil box (above): at 1.5 on 3440 px, up to 2.5 px at the right edge, growing across x; 1440 / 1.5 is whole, so y is exact.
- **The last column or row at such a scale can give no position.** The pointer reaches niri's ceil box, so in its last logical column the session can report a position at or past the output's physical size: at 1.5 on 3440 px, column 2293 (where niri stops a relative move) reports 3440 or 3441. Where the rounding stays inside, as at 1.4 on 1920 px (column 1371 reports 1919), the position is kept. A report past the edge is dropped, since it is also what this output's session says while the pointer is on the neighbour to its right and the cursor's image still overlaps this output; the two cannot be told apart. So the pointer in such a column records no position, and the movement into it plays as mouse movement: a click there lands where that movement takes the pointer, and one in a recording that has no position before it lands wherever the pointer is when the click plays. Part of the ceil difference accepted with no workaround (W01 decision 8).
- **Hyprland offers the same globals,** but its sessions report truncated logical px and its locked pointer drifts (Route B, below), so the program uses cursor sessions only where neither KWin nor Hyprland reports the pointer, finds Hyprland by its socket even without `HYPRLAND_INSTANCE_SIGNATURE`, and opens none while Hyprland could not be ruled out at start (a socket someone listens on that could not be checked, or a `hypr` directory that could not be read).

### Measured on niri-git (W02, 2026-11-17)

The dev box with niri-git 689c1e9, two outputs side by side: HDMI-A-1 (3440x1440 at 85 Hz) and DP-4 (3440x1440 at 144 Hz) to its right. Tools under the worktree's `.claude/local/w02` (`hm`, from W01c's).

- **Replay.** A virtual mouse puts the pointer at a random place on one output; the cursor session's position goes through the program's mapping and playback's formula, and the session reports where the pointer then lands. 300 places per case, |landed − reported| per axis in physical px: 0 at most on HDMI-A-1 and on DP-4 at scale 1; at most 1 on HDMI-A-1 at 1.25 and at 1.5 (most were 0). With DP-4 beside it, niri placed DP-4 at x = 2294 at 1.5, so its box and the reported one were the same and no ceil gap showed. The scale was set back to 1.
- **Delay.** A position followed each move 0.05 ms after it at the median, 0.20 ms at the 99th percentile and 0.74 ms at most (3,000 moves at 1 kHz, none missed); 0.11 / 0.26 / 0.42 ms at 125 Hz. niri sends it after each pass of its event loop, not on the output's frame as Hyprland does, so it is well inside R03's 2 ms delivery delay and 5 ms cover window.
- **A game holding the pointer.** With `lockwin.py` (SDL 3, relative mode, fullscreen) holding the pointer, 2,000 relative moves reached the window and no position came. Then 20 absolute moves gave the window no motion but moved the pointer, across both outputs (20 positions), and left no window focused: niri does not ignore absolute moves under a lock (F4 above). No window was focused because the dev box's config turns on `focus-follows-mouse`, which focuses the output under the pointer, and nothing was focused on DP-4 (`src/niri.rs:7216`, read from source; DP-4 was unplugged by W04, so this was not measured again). W04 below measures the lock and focus apart.
- **The daemon.** The release build recorded two clicks made with a virtual mouse, the first without moving after the hotkey started the recording: version 2, positions (945, 439) and (2100, 1099), each just before its press, as the device values predict on the 6880 x 1440 layout. Played back from elsewhere, both presses landed on those pixels in the probe window. `niri msg casts` said "No screencasts." while it recorded.
- **Idle,** 60 s (`scripts/measure-daemon.py`): 0 CPU ticks, 0 context switches, 1 thread, 8.1 MiB resident, as before. The daemon is 6,388,168 bytes (main 6,257,112: +131 KB, the staging protocols and the cursor sessions).

## niri: absolute moves while a window holds the pointer (W04)

Read from source at niri 689c1e9 (`src/input/mod.rs`); main at f73c145 has not changed it. Measured on the dev box on 2026-11-18: niri-git 689c1e9, HDMI-A-1 alone (3440x1440, scale 1; DP-4 unplugged), the owner's config with `focus-follows-mouse` on (niri's default config has it off), SDL 3.4.20, xwayland-satellite 0.8.3. Tools and outputs under the main checkout's `.claude/local/w04` (`results/`): `hm seq` (W02's `hm` with one virtual mouse, relative and absolute axes and a button, driven from stdin, logging a cursor session's positions), `lockwin2.py` (W01d's `lockwin.py`, optionally windowed or unlocked, printing SDL's focus flags and presses), and the scripts below, each run as `python3 <script> hm/target/release/hm lockwin2.py …` with `NIRI_SOCKET`, `WAYLAND_DISPLAY=wayland-1` and, for X11, `DISPLAY=:0`.

- **From source.** Relative motion checks the pointer focus's active constraint and, when it is a lock, sends only relative motion and returns (`src/input/mod.rs:2811-2858`). Absolute motion from an input device (`:217`) has no such check (`on_pointer_motion_absolute`, `:3032`): it finds the surface under the new position (`:3069`), runs focus-follows-mouse (`:3071`; with the option on, it focuses the output and the window under the pointer, `src/niri.rs:7216`), and calls `pointer.motion` (`:3075`), which moves the pointer and sends `wl_pointer.motion` to the locked surface, or, off it, moves the pointer focus, which deactivates the lock. A persistent lock is activated again when the pointer comes back onto its surface (`maybe_activate_pointer_constraint`, `:3100`). A press activates the window under the pointer, or focuses the output under it, with no lock check and whatever the focus-follows-mouse option says (`on_pointer_button`, `:3419`, `:3436`). pointer-constraints forbids the motion events: "While the lock of this object is active, the wl_pointer objects of the associated seat will not emit any wl_pointer.motion events" (`pointer-constraints-unstable-v1.xml:188-189`). KWin returns before moving a locked pointer (decision 0001, W01d below); Hyprland keeps absolute motion from the window (`blocksAbsoluteMotion`, below).
- **On the window, fullscreen** (`measure.py … fullscreen`; the window locked at (500, 700)). 3 absolute moves to (688, 432), (2409, 865) and (1720, 720) moved the pointer (3 positions) and gave the window no motion; it kept keyboard and pointer focus, and the next 500 relative moves all reached it with no position. With `WAYLAND_DEBUG=1` the window logged one `wl_pointer.motion` per absolute move while locked, at those positions. SDL in relative mode ignores them, but when the window let the pointer go it came back at the last absolute move plus the relative motion since, not at the lock spot: (720, 720) after 500 moves of -2, (1520, 720) after 100 (`REL_BACK=100`).
- **Off the window, windowed** (`measure.py … windowed`; 800 x 600, floating at (1320, 437)). 2 absolute moves to empty desktop took the pointer off the window: it lost pointer focus and kept keyboard focus, and the next 500 relative moves moved the desktop's pointer (100 positions, then the screen's edge) with none reaching the window. One absolute move back onto it took the lock again: the next 500 relative moves all reached it, no position. With the 2 moves onto another window instead (`OUTSIDE="300,300 600,1000"`), focus-follows-mouse gave that window keyboard focus and the locked window lost both, until the relative moves carried the pointer back across it (201 of 500 reached it).
- **A press off the window** (`click.py`: an unlocked 1600 x 1000 window, then the locked window over its middle). A press on the locked window reached it. An absolute move to (980, 297), on the other window, gave that window keyboard focus (focus-follows-mouse); the press there reached that window, not the locked one, and the next 200 relative moves moved the pointer over that window. Without focus-follows-mouse the press would still activate that window (source, above); not measured.
- **An X11 window** (`DRIVER=x11 measure.py … fullscreen`: SDL 3 under xwayland-satellite). It kept the lock through the absolute moves (no position from the relative moves after), but received the jumps between them as motion: 2 events summing (+1032, +288), the steps from (688, 432) to (2409, 865) to (1720, 720), in 3 of 3 runs; the first jump, from the lock spot, was not counted, unexplained. Indicative only: in this harness the X11 window's input was erratic (`x11check.py`, 6 runs: in 2 it took the pointer only after 730 and 1,230 px of motion, and it received 1 to 293 of 600 relative moves, with sums that do not match them).
- **The cursor sessions under a lock** sent no `enter`, `leave` or `hotspot` and no position while relative moves went in, and kept reporting absolute moves. With the pointer against the screen's edge, relative moves outward give no position either (400 of the 500 above).
- **niri's IPC tells of no lock:** at 689c1e9 its requests and events cover outputs, workspaces, windows, layers, keyboard layouts, the overview, casts, screenshots and the touchpad, nothing about pointer constraints. No protocol tells one client of another's lock. So playback could tell a lock only by sending a relative move and hearing no position after it, which an edge or a confinement also gives, and which says nothing before the first move.

### Recordings played into a window holding the pointer (W04, measured)

`recplay.py DAEMON …` (`MODES=windowed` for the windowed rows with focus logged from `niri msg event-stream`). The daemon, built from main at 36012ed, recorded a desktop recording: 400 relative moves of (+5, +1) from (700, 700), no clicks, version 2 with 372 positions from (700, 700) to (3439, 1439). Its version 3 copy is the same file with `"positions_from":"hyprland"`. Each was played by a daemon started for that play into `lockwin2.py` locked at (1720, 720). Then a recording made inside the window (200 moves of (+2, 0)) was played into it locked at the same spot and at another. Window counts are read once they stop changing; no clicks were played.

| Recording | Window | Window's motion from the play | Focus | Positions during the play |
| --- | --- | --- | --- | --- |
| Desktop, version 2 | fullscreen | 29 moves (145, 29): the moves no position covers | kept, locked | 372, across the output |
| Desktop as version 3 | fullscreen | 400 moves (2000, 400) | kept, locked | 371 |
| Desktop, version 2 | windowed | 0 | went to T3 Code, then back as the path crossed the window; pointer off it at the end | 372 |
| Desktop as version 3 | windowed | 0 and 43 of 400 in two runs | the same | 378, 390 |
| Desktop, version 2 | X11 fullscreen | 169 events (1681, 337): 29 moves (145, 29) and 140 from positions (1536, 308) | kept | 372 |
| Desktop as version 3 | X11 fullscreen | 171 events (855, 171), 171 of its moves; no jumps showed | kept | 382 |
| Made in the window: version 2, 1 position (1000, 500) | fullscreen, locked at (1000, 500) | 200 moves (400, 0) | kept, locked | 1 |
| The same | fullscreen, locked at (2500, 1000) | 200 moves (400, 0) | kept, locked | 1 |
| Made in the X11 window: 5 positions, (1720, 720) to (1731, 720) | X11, same spot | 196 events (392, 0) | kept | 5 |
| The same | X11, pointer at (2500, 1000) before it opened | 196 events (392, 0) | kept | 5 |

- **What takes a game's pointer out** is a position off the game's surface while it holds the pointer: the pointer leaves, the lock is gone until a position brings it back, the moves meanwhile go to the desktop, a press lands on the window under the pointer and activates it (measured with focus-follows-mouse; from source without it), and with focus-follows-mouse the window under the pointer takes keyboard focus as soon as the pointer reaches it. That happens when a desktop recording of either version is played into a game whose surface does not cover where the recording's positions go: a game in a window, or on another output. On Plasma the same plays leave the pointer where the game locked it (W01d).
- **On the game's surface, nothing unfocuses it,** and a Wayland game gets the same moves it would on Plasma: the moves no position covers for version 2, every move for version 3. The pointer still moves under it, so it comes back elsewhere when the game lets go, and the X11 window received jumps between positions as motion in `measure.py` and in the version 2 play (indicative, above).
- **A recording made inside the game** holds one position, where the locked pointer was (W02), or a few near it for the X11 window, whose lock took hold late. Played into the same game it kept focus and every move, also when the game had locked elsewhere on its surface: the one position moves the pointer within the game. That holds only while the recorded position is on the game's surface: when the game's window has moved or shrunk since, or the game runs on another output, so that the position is off its surface, the position takes the pointer out, as above.

### For an upstream report to niri (the owner files it)

- **Reproducer:** niri 689c1e9 (main f73c145 unchanged). An SDL 3 window in relative mouse mode (`SDL_SetWindowRelativeMouseMode`, which locks with `zwp_locked_pointer_v1`, persistent), fullscreen or windowed. A uinput device with `ABS_X`/`ABS_Y` (0..65535) sends absolute moves.
- **Observed:** each absolute move moves the locked pointer and sends the locked surface `wl_pointer.motion`; a move off the surface ends the lock and moves pointer focus (and keyboard focus with `focus-follows-mouse`); when the client lets go, the pointer reappears where the absolute moves left it. An X11 client under xwayland-satellite received the jumps as motion.
- **Expected:** while the lock is active, no `wl_pointer.motion` and the pointer stays where it is (`pointer-constraints-unstable-v1.xml:188-189`); the cursor position hint decides where it reappears. KWin and Hyprland ignore absolute motion under a lock.
- **Where:** `on_pointer_motion_absolute` (`src/input/mod.rs:3032-3100`) has no counterpart of the lock check in `on_pointer_motion` (`:2811-2858`).
- **Where it hurts:** any absolute pointer device used while a game holds the pointer; here, replayed recordings: a game in a window loses the pointer and its moves, and a press lands on another window.
- **Already reported?** No issue on 2026-11-18 (niri-wm/niri issues, pull requests and discussions for locked pointer, pointer lock, pointer constraint, absolute motion, lock_pointer, virtual pointer). Open pull request #4411 ("Keep pointer constraints in sync with keyboard focus", opened 2026-08-09, updated 2026-11-08) holds the fix, an early return in `on_pointer_motion_absolute` while a lock is active, with a test (`absolute_motion_does_not_end_active_lock`), inside a larger rework that waits on a Smithay fork commit.

## Hyprland, from source (v0.57.2)

- **Mapping.** An absolute pointer maps over the bounding box of every monitor's logical box (`src/pointer/PointerManager.cpp:941-956`) unless the user binds the device to an output. A monitor's logical size is round(physical/scale) (`src/output/Monitor.cpp:644`), the value xdg-output sends (`src/protocols/XDGOutput.cpp:127`). Hyprland refuses scales that leave fractional logical pixels (`Monitor.cpp:944`).
- **`cursorpos`.** It answers with floored whole logical px in global coordinates (`src/ipc/s1/Commands.cpp:1384`), over `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`, one connection per request.
- **Cursor sessions.** The position is the pointer's logical position less the output's corner, sent as the protocol's `int`, so truncated to whole logical px, not the buffer px the protocol names (`src/protocols/ImageCopyCapture.cpp:333-357`; measured below). It is sent on each monitor commit, so at most once per frame. A cursor move schedules a frame except under VRR fullscreen with `cursor:no_break_fs_vrr` (`Monitor.cpp:1109-1121`, `PointerManager.cpp:476`). It is allowed unless `ecosystem:enforce_permissions` is on (default off, `src/config/values/ConfigValues.cpp:731`), in which case Hyprland asks the user.

## Hyprland measurements (W01c, 2026-11-13)

Hyprland 0.57.2 (11cbc6c, aquamarine 0.16.0) from the CachyOS repositories, started with `start-hyprland` from a text console beside the desktop already running, with the generated default configuration. Linux 7.3.3-1-cachyos on both machines.

- **Dev box:** AMD Radeon RX 9070, one output, HDMI-A-1 (LG 34GN850, 3440x1440 at 85 Hz). A second output, where needed, was a headless one (`hyprctl output create headless`).
- **Laptop:** Intel Raptor Lake with an RTX 4080 Laptop GPU; eDP-1 (BOE NE160QDM-NM4, 2560x1600 at 240 Hz) and DP-1 (the same LG, 3440x1440 at 143.92 Hz). Hyprland's own choice of scale for eDP-1 was 1.6.
- **The tools** are under `.claude/local/w01c/` in the main checkout:
  - `hm`, a Rust client: wayland-client 0.31.15, wayland-protocols 0.32.13 with `staging`. It makes a virtual mouse shaped like C01's on `/dev/uinput`, reads its event node with `CLOCK_MONOTONIC` kernel stamps, asks `cursorpos` over a new connection each time, and reads xdg-output with W01a's rule for a whole layout (a `wl_display.sync` answered with nothing new heard and every output's `done` in). It also opens one cursor session per output.
  - `hmap.py` drives the mapping runs with the GTK 4 probe from X01 (`probe.py`).
  - `lockphases.py`, `lockgame.py` and X01's `lockwin.py` (an SDL 3 window in relative mouse mode) drive the pointer-lock runs.
  - Layouts were set with `hyprctl eval 'hl.monitor({ output = …, mode = …, position = …, scale = … })'`. Hyprland 0.57's configuration is Lua and `hyprctl keyword` is gone. Every layout was set back after its run.

### Mapping and layout (measured)

An 8x8 grid of absolute values (0, 1, 1000, 16384, 32768, 49152, 64000 and 65535 per axis). A fullscreen probe on each output gave where it saw the pointer, its corner taken from `hyprctl clients -j`. Predicted is `box + v * W / 65536` on the bounding box of xdg-output's layout. `cursorpos` was read beside it.

| Machine | Layout | Box | Worst error (logical px) |
| --- | --- | --- | --- |
| Dev box | HDMI-A-1 @1 | 3440x1440 | 0.002 |
| Dev box | HDMI-A-1 @1.25 | 2752x1152 | 0.002 |
| Dev box | HDMI-A-1 asked for 1.5; Hyprland applied 1.6 | 2150x900 | 0.002 |
| Dev box | HDMI-A-1 @1 at (-500, -300) | 3440x1440 | 0.002 |
| Dev box | HDMI-A-1 @1.25 at (-500, -300) | 2752x1152 | 0.002 |
| Dev box | headless 1920x1080 @1 at (-1920, -300), HDMI-A-1 @1 at (0, 0) | 5360x1740 at (-1920, -300) | 0.002 |
| Dev box | headless @1.25 at (-1536, -400), HDMI-A-1 @1.25 | 4288x1552 at (-1536, -400) | 0.002 |
| Laptop | eDP-1 @1.25, DP-1 @1 right | 5488x1440 | 0.002 |
| Laptop | DP-1 @1 at (-3440, -200), left of eDP-1 @1.25 | 5488x1480 at (-3440, -200) | 0.002 |
| Laptop | eDP-1 asked for 1.5 (1.6 applied), DP-1 @1.25 above at (0, -1152) | 2752x2152 at (0, -1152) | 0.002 |
| Laptop | eDP-1 @1.25 alone (DP-1 disabled) | 2048x1280 | 0.000 |

- **Hyprland maps the absolute device over the bounding box of every output's logical box, wherever it lies,** as the source says. Decision 0001's formula holds on the box moved to (0, 0), negative positions included.
- **xdg-output matched `hyprctl monitors -j`** (position, and round(physical/scale) as the size) for every output in all eleven layouts.
- **A scale Hyprland refuses is replaced:** 1.5 on a 3440 or 2560 px wide output became 1.6. The scales Hyprland applies leave whole logical sizes, so the box, xdg-output and the mapping agree. niri's 1 px rounding gap (above) does not arise.
- **The pointer can rest in no output.** A point of the box that lies on no output (the empty corner of two outputs of different heights) puts the pointer there, with no clamping: the probe, which still receives the motion, read it within 0.002 logical px of the formula, and `cursorpos` gave the formula's value floored (for the first two-output layout, a predicted x of -1919.918 read as -1920). 8 to 28 grid points per two-output layout landed so.

### Route A: `cursorpos` (measured)

The virtual mouse moved every 1 ms (a 1000 Hz mouse), every 8 ms (125 Hz) or every 0.25 ms. Each move went to a point whose floored position differs from the move before. `hm routea` read each move from the event node and asked `cursorpos` at most once per ms, with one last ask at the end of the cap for a move it held back. Each ask used a new connection to `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`, wrote `cursorpos` and read `x, y` (whole logical px, floored, global) to EOF. "Delay" is from the move's kernel stamp to the first answer that describes it or a later move, R03b's measure.

| Machine, load | Moves | Stale answers | Delay median / p99 / p99.9 / max (ms) | Over 2 ms | Over 5 ms |
| --- | --- | --- | --- | --- | --- |
| Dev box, idle, 1 kHz | 5,000 | 0 of 4,758 | 0.56 / 1.06 / 1.11 / 1.19 | 0 | 0 |
| Dev box, idle, 125 Hz | 600 | 0 of 600 | 0.07 / 0.16 / 0.25 / 0.53 | 0 | 0 |
| Dev box, idle, 4 kHz | 8,000 | 0 of 1,902 | 0.55 / 1.06 / 1.11 / 1.67 | 0 | 0 |
| Dev box, VRR fullscreen window, `no_break_fs_vrr = 1` | 3,000 | 0 of 2,857 | 0.51 / 1.04 / 1.05 / 1.07 | 0 | 0 |
| Laptop, idle, 1 kHz | 5,000 | 0 of 4,647 | 0.76 / 2.02 / 2.35 / 2.50 | 59 | 0 |
| Laptop, idle, 125 Hz | 600 | 0 of 600 | 1.13 / 1.63 / 2.22 / 3.66 | 3 | 0 |
| Laptop, Minecraft rendering its pause menu, 1 kHz | 10,000 | 0 of 9,211 | 0.77 / 2.25 / 2.86 / 3.08 | 224 | 0 |
| Laptop, Minecraft rendering its pause menu, 125 Hz | 1,000 | 0 of 1,000 | 1.42 / 2.22 / 2.47 / 2.70 | 37 | 0 |

- **No answer described the pointer before the move behind it,** idle or under the game. Answers newer than the move read (up to 19 % of answers at 1 kHz under the game) are moves that arrived while an ask waited for the cap. That Hyprland always handles a move before a request that follows it is our inference from the numbers, not read from source.
- **Against R03's windows,** the laptop under the game gives 2.2 % of moves over the 2 ms delivery delay and none over the 5 ms cover window. R03b measured KWin's script on the same laptop at 0.81 ms median, 1.91 ms at p99 and 2.19 ms at p99.9, with 28 of 9,756 moves (0.29 %) over 2 ms and one over 5 ms. So Route A meets the 5 ms cover window as KWin's script does, and exceeds the 2 ms delivery delay on more moves (2.2 % against 0.29 %). Under R03's pairing rule, a press that follows such a move pairs with no position of its own; clicks themselves were not measured here.
- **Hyprland's cost per ask:** 20,000 asks back to back took 5.4 µs of Hyprland's CPU each on the dev box and 13.2 µs on the laptop (`/proc/<pid>/task/*/schedstat`). The round trip from the client was 8 to 19 µs (median). At the cap during sustained 1 kHz motion, the asks added 38 to 57 ms of Hyprland's CPU per 5 s on the dev box (about 1 % of one core).
- **Frame time with a game running:** see "Route A's cost in a game" below.

### Route B: cursor sessions (measured)

`hm routeb` bound `ext_output_image_capture_source_manager_v1` and `ext_image_copy_capture_manager_v1` (version 1 of both on Hyprland 0.57.2), created a source and a pointer cursor session for each output, and made the same moves. Positions came without a capture session (`get_capture_session`), and the same with one. No permission prompt appeared (`ecosystem:enforce_permissions` is off by default).

| Machine, case | Moves | Positions | Delay median / p99 / max (ms) | Over 2 ms | Over 5 ms | Moves never reported |
| --- | --- | --- | --- | --- | --- | --- |
| Dev box, 85 Hz, 1 kHz moves | 3,000 | 257 | 5.96 / 11.71 / 11.96 | 2,504 | 1,740 | 0 |
| Dev box, 85 Hz, 125 Hz moves | 600 | 409 | 5.94 / 11.64 / 11.72 | 503 | 354 | 0 |
| Dev box, fullscreen window with VRR, `no_break_fs_vrr = 1` | 2,000 | 42 | 23.18 / 46.34 / 49.20 | 1,858 | 1,735 | 59 |
| Dev box, the same with `no_break_fs_vrr` 0 or 2 (default) | 2,000 | 172 | 5.92 / 11.71 / 11.87 | 1,670 | 1,161 | 0 |
| Laptop, eDP-1 alone at 240 Hz, Minecraft rendering its pause menu, 1 kHz moves | 3,000 | 721 | 3.13 / 5.25 / 6.68 | 2,327 | 176 | 0 |
| Laptop, the same, 125 Hz moves | 600 | 600 | 1.99 / 6.96 / 7.15 | 298 | 27 | 0 |

- A position is sent on the output's commit, at most once a frame (11.76 ms apart at 85 Hz, 4.16 ms at 240 Hz), so it comes about half a frame after the move, up to one frame.
- A session reports only while the pointer overlaps its output (`sendCursorEvents`, read from source): with the pointer in no output (above), no session reports. The first laptop runs, over two outputs, swept that empty corner too, and are left out.
- The position is in whole logical px, truncated (all 205 positions at scale 1.25 matched logical, not buffer, px), from the output's corner. It carries its output, so it carries its layout.
- Under VRR fullscreen with `cursor:no_break_fs_vrr = 1`, a move schedules no frame: positions came about every 47 ms (`cursor:min_refresh_rate` is 24 Hz, 41.7 ms) and 59 moves never got one. `misc:vrr` and `no_break_fs_vrr` were set back to 0 and 2 after.
- **Route B misses R03's windows** at 85 Hz and at 240 Hz: about half or more of the moves wait longer than the 2 ms delivery delay, and some longer than the 5 ms cover window.

### A game holding the pointer (measured, and the cause from source)

Relative moves at 1 kHz with varied steps (dx -6..6, dy -3..3), both routes on, Route A capped as above. "Covered" counts moves that a changed answer or position followed within 5 ms. Only a change can cover a move: the daemon keeps a position only when it differs from the last, as KWin's script reports only a move.

| Window holding the pointer | Locked moves | Route A: distinct answers, spread | Route A: moves covered | Route B: moves covered |
| --- | --- | --- | --- | --- |
| None (control, dev box) | 3,515 | 3,074 | 3,514 (100 %) | 297 changes in 298 positions |
| `lockwin.py`, SDL 3 on Wayland, fullscreen (dev box; it received 3,670 motion events over the run, the moves while it took the pointer included) | 2,805 | 28, within 23x13 px | 180 (6 %) | 0 (1 position) |
| `lockwin.py`, constant steps of ±2 (dev box) | 3,000 | 3, within 9 px | 3,000 (100 %) | 8 positions, all changes |
| Minecraft Java 26.3, SDL 3 on Wayland, fullscreen on eDP-1 at 240 Hz (laptop) | 10,000 | 54, within 25x13 px | 507 (5.1 %) | 203 (2.0 %) |

- **Hyprland's pointer does not stay at the lock point.** Every relative move first moves the pointer (`CInputManager::onMouseMoved` → `CPointerManager::move`, `src/managers/input/InputManager.cpp:158`). Then `mouseMoveUnified` warps it back to the lock's hint (`PointerConstraintController::apply`, `src/managers/input/PointerConstraintController.cpp:190-193`). But `mouseMoveUnified` returns early when the floored position equals the last one it handled (`InputManager.cpp:259-260`). After a warp, the next move often lands on the pixel the move before reached, so its warp is skipped. The pointer then rests one move away from the hint until a later move warps it back. `cursorpos` and the cursor session read that resting place.
- **How often a locked move is covered depends on the steps:** 5 % in Minecraft with varied steps, 100 % with steady steps of 2. A real mouse was not measured.
- **Nothing outside Hyprland says a lock holds.** Its event socket and `hyprctl` expose no pointer lock (its events are window, workspace and monitor events), on v0.57.2 and on main (8333bb7, 2026-11-12), where the early return is unchanged. A request for a `pointerlock` event (hyprwm/Hyprland discussion #16512, 2026-10-09) has no answer. In most runs a client sees only answers that stay within a few px of one point while the mouse keeps moving. In one (`lockabs.py`, steady steps of ±3, 2,000 moves with no net motion), `cursorpos` ended 568 px from where it began while the window still held the pointer and received every move; that was not explained.
- An X11 window (SDL 3 with `SDL_VIDEO_DRIVER=x11`) did not take the pointer under Hyprland's XWayland: it received no motion and was not fullscreen. Minecraft under XWayland was not measured.

### W4's bound: the layout notice against positions (measured)

`hm w4` moved the pointer between two absolute values every 1 ms, asked `cursorpos` back to back (every 0.25 ms) and kept the cursor sessions open, while `hyprctl eval` changed the layout and changed it back. For each change it found the first answer that fits only the new layout and compared it with the moment W01a's rule published the new layout.

| Machine | Change (repeats) | Publish after the command (ms, max) | Publish after the new answer's arrival (ms, max) | Publish after that ask was sent (ms, max) |
| --- | --- | --- | --- | --- |
| Dev box | scale 1 ↔ 1.25 (20) | 31.8 | 0.064 | 29.3 |
| Dev box | (0, 0) ↔ (-500, -300) (20) | 9.4 | 0.028 | 1.1 |
| Dev box | headless output added ↔ removed (20) | 10.1 | 0.105 | 10.1 |
| Laptop | DP-1 scale 1 ↔ 1.25 (20) | 20.5 | 1.24 | 18.9 |
| Laptop | DP-1 right ↔ left of eDP-1 (20) | not kept | 2.48 | not kept |
| Laptop | DP-1 off ↔ on (8) | 3,220.6 | 3.12 | 2,657.6 |

- Hyprland holds `cursorpos` while it applies a change: an ask sent during a modeset is answered only after it, with the new layout's position. DP-1's modeset held it for up to 2.66 s.
- An answer on the new layout arrived before the layout's publish by at most 3.12 ms (laptop, DP-1 back on), and never after it by more than a fraction of a ms: no answer fitting only the old layout came after a publish.
- **The bound holds for a position stamped when its answer is read,** as the recorder stamps KWin's positions (`crates/backend-kwin/src/receiver.rs`): 3.12 ms at most over 108 changes (8 of them DP-1 off and on). With a margin of about three times that, **10 ms**. Stamped at the ask or at the move, the bound would be the length of a modeset, which has no limit.
- Route B's positions on the new layout came before the layout's publish 31 times over these runs, on the same connection. A position is from its output's corner, so pairing it with that output's own geometry at the time is enough: it needs no bound.

### Route A's cost in a game (measured)

Minecraft Java 26.3 from the official launcher (`minecraft-launcher.sh`), singleplayer, on eDP-1 (2560x1600 at 240 Hz, scale 1.6; `hyprctl` reported the window fullscreen while it was played, and not fullscreen during the pause-menu blocks below), native Wayland (`latest.log`: `Created window using SDL video driver: wayland`). MangoHud (`mangohud --dlsym`) was loaded into the game but logged no frames, so Hyprland's own frame interval was measured instead. With the game rendering its pause menu, the virtual mouse moved every 1 ms in six 20 s blocks, alternately without asks and with Route A at its cap (about 18,300 asks a block). Beside it, `hm frames 21` held a cursor session, which reports at each commit of the output while the pointer moves, and timed the gaps.

| Blocks | Commits | Interval median / p99 / p99.9 (ms) | Over 5 ms | Over 6 ms | Over 9 ms (a frame missed) |
| --- | --- | --- | --- | --- | --- |
| Without asks (3 x 20 s) | 14,325 | 4.16 / 4.76 to 4.86 / 5.21 to 7.59 | 50 | 11 | 0 |
| With Route A (3 x 20 s) | 14,339 | 4.16 / 4.80 to 4.96 / 5.25 to 5.48 | 68 | 3 | 0 |

- **Route A costs Hyprland no frame:** the interval stays at 240 Hz's 4.17 ms either way, and no frame was missed. The one gap of about 1.1 s in each block, before the moves begin, is left out.
- Hyprland's CPU per 20 s block was 9,467, 8,928 and 8,809 ms without asks, and 9,873, 9,861 and 10,065 ms with them: about 870 ms more a block, 4.3 % of one core, or 47 µs an ask under this load. While the game held the pointer and the virtual mouse turned the view (`hm routea 1000 20000 rel`, with and without `noask`), the same comparison gave 7,571 against 8,441 ms (mean of three each).

### Absolute moves while a window holds the pointer (measured)

`lockabs.py` on the laptop (game closed): `lockwin.py` held the pointer (SDL 3, Wayland, fullscreen), then 2,000 absolute moves and 2,000 relative moves went in. The window counted 0 motion events from the absolute moves and 2,000 from the relative ones, and `cursorpos` stayed at the lock point through the absolute moves. Measured: absolute moves during a lock gave the window no motion, and `cursorpos` read the same before and after the 2,000 moves. From source, `blocksAbsoluteMotion` (`SeatManager.cpp:421`) keeps the motion from the window, while the pointer itself is moved and warped back like a relative move, so the same early return could leave it moved for a while; this run neither showed nor rules that out. The game gets no motion from a played-back position either way.

### Finding Hyprland (W03, 2026-11-18)

Checks of how the daemon finds Hyprland since W02: first the socket that `HYPRLAND_INSTANCE_SIGNATURE` names, then the newest directories under `$XDG_RUNTIME_DIR/hypr`, and an unsure result that opens no cursor sessions. They ran on the laptop, logged into Hyprland 0.57.2 (11cbc6c) alone at the login screen, on Linux 7.3.4-1-cachyos, with eDP-1 alone at scale 1.6 (a 1600x1000 layout). The daemon was the release build of main at 36012ed (sha256 `a7d6d04f…`). The installed service was stopped during the checks.

The tools are under `.claude/local/w03/` in the main checkout. `run-all.sh` runs `case.py` once per case. Each case runs the daemon with its own config and data directories and `LANG=C.UTF-8`, and makes a virtual mouse and keyboard (W01d's `devices.py`). It reads the tray's `ToolTip` over D-Bus at idle and while recording. It records 150 relative moves with Ctrl+Shift+F7, with no clicks. The script also counts the daemon's connections to the display: Hyprland's ends of the display socket (`ss -xH`) whose peer is one of the daemon's sockets. Then it reads the recording and the log.

The cases need no other setup. For a stale signature with no directory, the script names `<commit>_1700000000_123456789` under `hypr`. For a leftover instance, it creates `hypr/<commit>_1700000001_w03stale/` and binds `.socket.sock` in it without listening, closing the socket without unlinking it, so a connect is refused. For an unreachable socket, it runs `chmod 000` on the instance's `.socket.sock` before the daemon starts and restores the old mode once the log shows the detection line. To run every case: copy `case.py`, `devices.py`, `run-all.sh` and the binary to `/tmp/w03` on the Hyprland machine, then run `bash /tmp/w03/run-all.sh`, which writes `/tmp/w03/out.txt`. To run one case, stop the installed service and run, for example, `python3 /tmp/w03/case.py set`. The other cases are `unset`, `stale-missing`, `stale-dead`, `chmod` and `chmod-unset`.

As a control, the counter was run on niri, where cursor sessions open. It read 1 connection at idle and 2 while recording, so it can tell the two apart.

| Case | Daemon's `HYPRLAND_INSTANCE_SIGNATURE` | Log | Tooltip | Display connections, idle / recording | Recording |
| --- | --- | --- | --- | --- | --- |
| Set | the running instance | nothing about Hyprland | no problem | 1 / 1 | version 3, `"positions_from":"hyprland"`, 161 positions for 150 moves |
| Unset | removed | nothing about Hyprland | no problem | 1 / 1 | version 3, Hyprland, 159 positions |
| Stale, no such directory | `…_1700000000_123456789` | nothing about Hyprland | no problem | 1 / 1 | version 3, Hyprland, 157 positions |
| Stale, left behind | a fake instance made for the case, newer than the running one, holding a bound socket nobody listens on | nothing about Hyprland | no problem | 1 / 1 | version 3, Hyprland, 156 positions |
| Socket unreachable | the running instance, its `.socket.sock` at mode 000 until the daemon logged its detection, then put back (`hyprctl` answered again) | "a Hyprland socket could not be checked against this daemon's Wayland display, so neither Hyprland nor the display's cursor sessions give positions" | "Recordings keep movement only: Hyprland's socket did not answer", at idle and while recording | 1 / 1 | version 1, movement only: 150 moves, no positions, no layout |
| The same, unset | removed, the socket at mode 000 as above | the same line | the same | 1 / 1 | version 1, movement only |

- **Each case passed.** With a signature, without one, or with a stale one, the daemon picks the running Hyprland and records its route (version 3). Without a signature, or with a stale one, the scan finds it, so no "names no socket of this daemon's Wayland display" line is logged.
- **The unsure case,** reached here through `EACCES` on connect, stays unsure for the daemon's life: the socket was readable again before the recording, and the recording still kept movement only. It opens no second connection to the display while recording, so no cursor session stands in for Hyprland.
- **The fake instance** was removed after its case. The socket's mode was put back after each unreachable case. The laptop went back to Plasma afterwards.

### For an upstream report to Hyprland (the owner files it)

- **Reproducer:** on Hyprland 0.57.2 (and main, 8333bb7), an SDL 3 window in relative mouse mode, fullscreen (`zwp_locked_pointer_v1`, locked). A relative mouse sends REL_X = +2 every 1 ms for 500 moves, then -2 for 500 (the first version of `hm lock`; default pointer acceleration). `hyprctl cursorpos` takes three values, the lock point and 4 px either side (1037, 1041 and 1045, here), changing on 1,915 of 2,855 answers, instead of staying at the lock point.
- **Expected:** while a pointer is locked, the pointer stays where the lock put it (its position hint), so every move is warped back.
- **Where:** `CInputManager::onMouseMoved` moves the pointer by the delta (`src/managers/input/InputManager.cpp:158`, `Pointer::mgr()->move`). `mouseMoveUnified` returns when the floored position equals `m_lastCursorPosFloored` (`:259-260`), before `m_pointerConstraints.apply` (`:318`) warps it back (`PointerConstraintController.cpp:190-193`). After one warp, the next move lands on the pixel the last one reached and is left there.
- **Effect:** a locked pointer drifts, by a few px in most runs (once by 568 px, unexplained). Anything that reads the pointer (`cursorpos`, cursor sessions) sees it move while the game holds it. For this program, a position cannot tell a game's held moves from the desktop's.
- **Already reported?** No match on 2026-11-13 in Hyprland's issues, pull requests or discussions for locked pointer, pointer lock cursor position, `m_lastCursorPosFloored` or constraint warp. Discussion #16512 asks for a `pointerlock` event, a separate request.

## KWin 6.8.2: xdg-output against KWin's layout (measured)

On the laptop, `kscreen-doctor` set each layout below; 3 s later a raw Wayland client read every output's xdg-output position and size, and `qdbus6 org.kde.KWin /KWin supportInformation` gave KWin's own geometry per screen (the rounded rectangle the layout script reports). The original layout was restored after.

| Layout | KWin and xdg-output (both the same) |
| --- | --- |
| panel @1.25, DP-1 @1 right | eDP-1 0,0 2048x1280; DP-1 2048,0 3440x1440 |
| panel @1.25, DP-1 @1.5 right | eDP-1 0,0 2048x1280; DP-1 2048,0 2293x960 |
| DP-1 @1.5 left of the panel | DP-1 0,0 2293x960; eDP-1 2294,0 2048x1280 |
| DP-1 @1.5 above the panel | DP-1 0,0 2293x960; eDP-1 0,960 2048x1280 |
| panel @1.5 left of DP-1 @1 | eDP-1 0,0 1707x1067; DP-1 1707,0 3440x1440 |
| panel @1.5 above DP-1 @1 | eDP-1 0,0 1707x1067; DP-1 0,1067 3440x1440 |
| panel @1.25, DP-1 @1.25 right | eDP-1 0,0 2048x1280; DP-1 2048,0 2752x1152 |

Every output's rectangle was identical in both, fractional widths (2293.33, 1706.67 and 1066.67 logical) included. KScreen places outputs at whole logical positions, which is the case where the two roundings below agree.

From source:

- xdg-output sends round(size) and round(position) of an output's fractional geometry (`src/wayland/xdgoutput_v1.cpp:191,199`). The layout script's `output.geometry` rounds the edges instead (`src/core/rect.h:2439`). The two agree when an output sits at a whole logical position.
- KWin 6.8.2 has no ext-image-copy-capture.

## KWin 6.8.2: positions on D-Bus against xdg-output's notice (W01b, measured)

Positions come from KWin's position script over D-Bus and the layout from xdg-output over Wayland, so W4 needed to know how the two channels line up around a change. On the laptop (eDP-1 alone, 2560x1600), a test daemon built from main at f853cca stamped, on one clock (`CLOCK_MONOTONIC`): each position report, each report of the old layout script, and each layout the Wayland source published after its quiet round trip. A virtual relative mouse jiggled the pointer at the panel's right edge every millisecond, and `kscreen-doctor` switched the panel between scale 1.25 (2048x1280 logical) and 1.5 (1707x1067) four times, 3 s apart. A position on the new screens shows by its x: at 1.5 none reaches 1707.

| Run | New-screen positions ahead of xdg-output's notice | KWin's layout report ahead of xdg-output's |
| --- | --- | --- |
| debug build, idle | 33.5, 34.4, 34.4, 34.5 ms | 34.4 to 35.3 ms |
| release build, idle | 33.5, 34.2, 34.4, 35.0 ms | 33.6 to 35.0 ms |
| release build, `stress-ng` on every CPU, `--io 2 --vm 2` | 49.2, 50.5, 58.4 ms; once xdg-output came 200 ms first | 49.2 to 58.4 ms; that once, 200 ms after |

In the one loaded change where D-Bus ran 200 ms late, xdg-output's notice came first and positions taken on the old screens kept arriving for 200 ms after it. So no window measured on either side of xdg-output's notice bounds where a position belongs: how late D-Bus runs under load has no fixed limit. In every change, including that one, KWin's own layout report reached the daemon before its first position on the new screens (0.04 to 1.4 ms before), in order on one D-Bus connection. W01b therefore has the position script report the layout on every change of the screens, and judges each position by the last layout the script reported before it (W4's "a source whose positions carry their layout"). No timing constant is kept. A change that moves the pointer keeps that order too. With W01b's build on the laptop (eDP-1 at 1.25 beside DP-1 at 1, 3440x1440) and the pointer parked on DP-1, moving DP-1 down by 100 px and back, and disabling DP-1, each had the script report the new layout before the pointer KWin moved with it (0.3 to 1.2 ms before), and enabling DP-1 again reported the layout. xdg-output's notice came 2 to 4 ms after the script's report on a move, and 3.1 to 3.2 s after it on a disable or an enable, so a layout taken from xdg-output alone would have judged the moved pointer by the old screens for seconds. The order is not a guarantee, read from KWin 6.8.2's source: `Workspace::updateOutputs` clears its output list and adds the outputs back one at a time, each `setData` emitting that output's `geometryChanged` while the list holds only the outputs added so far (`workspace.cpp:1417-1438`), and `desktopResized` then warps the pointer (`:2380`) before `geometryChanged` and `outputsChanged` (`screensChanged`) go out. So the script also checks the layout as it is with each position, and reports it first when it differs from its last report. Read from KWin 6.8.2's source, a restarted KWin creates its xdg-output global (`wayland_server.cpp:442`, in `WaylandServer::init`, `main_wayland.cpp:656`) before it claims `org.kde.KWin` (`workspace.cpp:168`, `dbusinterface.cpp:47`, reached from `a.start()` at `main_wayland.cpp:711`), so a connection made on hearing the new name finds it. Measured with W01b's build: after `kwin_wayland_wrapper` was sent `SIGHUP` twice (`pkill -HUP -f '^/usr/bin/kwin_wayland_wrapper'`; `pkill -x` matches nothing, since the process name is cut to 15 characters), `kwin_wayland` ran under a new pid each time while the wrapper kept its pid and the socket; the daemon's connection read `Lost` at once, and the layout came back whole 3.4 s after the new KWin claimed `org.kde.KWin` (which it did 4 to 9 s after the old one left). A new KWin does not answer D-Bus at first: the position script's load into it timed out (2 s) every time. On main the layout script's load took that timeout and the position load, sent after it, succeeded; with the layout script gone, a recording that spans a restart kept no positions after it until W01b tried a failed position load again on the back-off. With that, the script's first report came 10.2 s after the old KWin left, against 9.6 s on main's build, and a recording started after the restart in the same daemon kept its positions. Raw logs: `.claude/local/w01b-spike/` in the main checkout, not committed.

## KWin 6.8.2: absolute moves while a window holds the pointer (W01d, measured)

A recording made on Hyprland sends every relative move and its positions wherever it plays (W01 decision 17), so on Plasma its positions reach KWin while a game may hold the pointer. Measured on the laptop under Plasma 6.8.2 (KWin 6.8.2) on 2026-11-13 with `kwinlock.py` (W01d's `.claude/local/w01d/`): `lockwin.py` (SDL 3, Wayland, fullscreen, relative mode) held the pointer while one virtual mouse shaped like C01's (relative and absolute axes, a button) sent 2,000 relative moves of +1, then 2,000 absolute moves across the whole range, then 2,000 more relative moves, each 1 ms apart. The window counted 2,000 motion events (summing to 2,000 px) after the first relative moves, none more after the absolute ones, and 4,000 after the second relative moves: KWin gave the window no motion from absolute moves and kept the lock, as its source says (`PointerInputRedirection::updatePosition` returns while locked, decision 0001). The window's own counts lag its input by a second or more, so each count was read after the next batch began.

## One change, every output's events before a sync answer (W01a review, from source)

xdg-output and `wl_output` end an update per output; nothing marks the end of a change across outputs. The client reads the layout when a `wl_display.sync` comes back with nothing new and every output's update ended at its `done` (W3). That is whole only if a compositor queues every affected output's events before it reads the client's next request. Read from source (smithay ff5fa7d as pinned by niri 8ed0da4, KWin v6.8.2 f130ab3, Hyprland v0.57.2 11cbc6c, libwayland f2ecaa6), it does on all three:

- **niri:** `reload_output_config` (`src/niri.rs:1718`) sets each output's scale and transform (`:1749`), resizes (`:1795`), applies modes and enables or disables outputs through the backend (`:1802`; `tty.rs:2495`, `:1545`, `:1601`) and repositions every output (`:1804`, `:2799`) in one call; smithay's `Output::change_current_state` (`src/output.rs:380`) sends the xdg-output and `wl_output` events and `done` at once (`src/wayland/output/xdg.rs:91-147`, `src/wayland/output/mod.rs:218-260`). Client requests are read only in `dispatch_clients` (`src/niri.rs:2465`). An output may get two `done`s in one change; both come before any answer.
- **KWin:** `Workspace::updateOutputs` (`workspace.cpp:1370`, loop at `:1435`) sets every output's data in one loop, reached synchronously from a configuration change (`:564-568`) and from hotplug (`:1328`); each change sends its `wl_output` and xdg-output events at once (`wayland/output_interface.cpp:209-265`, `xdgoutput_v1.cpp:115-148`) and schedules that output's `done` on a 0 ms timer (`:187-195`, `:322-326`). KWin flushes before it blocks (`display.cpp:128`), so a client can see every output's events before their `done`s, and an answer between them: the client's per-output `done` rule waits for them.
- **Hyprland:** a rule, configuration or output-management change only marks a reload, and the next render applies every rule and rearranges every output in one call (`MonitorRuleManager.cpp:22-28`, `:133-194`); a connect rearranges at once (`Monitor.cpp:118`, `:341`), a disconnect in an idle callback (`MonitorLayoutController.cpp:27`). Each output's `done` is deferred to an idle callback (`Monitor.cpp:1672`, `EventLoopManager.cpp:215`); libwayland drains every idle callback, new ones included, before the dispatch returns (`event-loop.c:969`, `:1064`), and Hyprland flushes clients only after that (`wayland-server.c:1740`). Two narrow exceptions: libwayland flushes a client whose buffer fills (`connection.c:594`, `:628`), and the retry after a failed mode set (`Monitor.cpp:1056`) applies one output's rule without rearranging the others, a layout the compositor itself holds. W01c's 108 layout changes on Hyprland each published once, whole (below).

A compositor that broke this would show one change as two layouts, each published whole per output and each firing, so a pick begun before either is refused and fixed-point clicking stops on the first; the Wayland source gives no positions, and a recording keeps only the layout it started on.

## The Wayland client's cost

A prototype ran on tokio's current-thread runtime, with wayland-client 0.31.15 and wayland-protocols 0.32.13 (the versions already in `Cargo.lock`; pure Rust, no libwayland) and the workspace's release profile. It bound every `wl_output` and its xdg-output and slept on the connection with `AsyncFd`. Against the same runtime without it, over 60 s idle on niri 26.04:

| | Binary | Resident | CPU ticks | Context switches | Threads |
| --- | --- | --- | --- | --- | --- |
| tokio alone | 443 KB | 2,700 KiB | 0 | 0 | 1 |
| with the xdg-output watcher | 645 KB | 2,940 KiB | 0 | 0 | 1 |

It reported the new layout at once when the scale changed to 1.5 and back. xdg-output v3 sends its `done` on `wl_output`, not on its own object.
