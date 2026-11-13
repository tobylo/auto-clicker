# 0004: The screen layout and the pointer on niri and Hyprland

- Status: accepted, 2026-11-12; Hyprland measured by W01c, 2026-11-13. niri and Plasma measured. W01a added niri at a negative position (one output); two outputs wait for a second monitor.
- Issue: W01 (#75). It shapes the slices in the [W01 brief](../planning/features/w01-other-desktops.md).
- Measured on: the dev box, niri 26.04 (8ed0da4), Linux 7.3.3-1-cachyos, one output (HDMI-A-1, LG 34GN850, 3440x1440 at 85 Hz); the laptop, KWin 6.8.2, eDP-1 (2560x1600) and DP-1 (3440x1440). Hyprland 0.57.2 on both machines (below). Read from source: niri v26.04 (commit 8ed0da4; 6a0a862 is its tag object) with its pinned smithay (ff5fa7d), Hyprland v0.57.2 (11cbc6c; main at 8333bb7 for the pointer lock), KWin v6.8.2.

## Decision

- **niri maps the absolute device over the bounding box of all outputs,** as KWin does, so decision 0001's formula holds there, with W and H the box's size. At a fractional scale where physical/scale is not a whole number, niri's box (ceil) can be one logical px larger than the size xdg-output reports (round), so a point lands up to 1 logical px off near the far edges. Accepted (W01 decision 8): no workaround.
- **xdg-output is the layout source on every desktop.** niri and Hyprland report each output's logical position and size there; on Plasma it matched KWin's own layout in every layout tried, so Plasma moves to it too (W01b). The daemon moves the bounding box to (0, 0).
- **niri gives no pointer position in 26.04.** Recordings there keep relative movement. ext-image-copy-capture cursor sessions, on niri's main branch, are the way to get it later (W02).
- **Hyprland maps the absolute device over the bounding box of all outputs** (measured on one and two outputs, negative positions included, at 1, 1.25 and 1.6; it replaces 1.5 by 1.6 where 1.5 leaves a fractional size). xdg-output matches `hyprctl monitors`, so decision 0001's formula holds on the box moved to (0, 0), within 0.002 logical px.
- **Hyprland's positions come from `cursorpos` (Route A),** asked on each mouse move at most once per ms plus one last ask: no stale answer in the runs measured (about 25,000 answers), inside the 5 ms cover window as KWin's script is, somewhat more often over the 2 ms delivery delay than KWin's script (2.2 % of moves against 0.29 %), and no frame lost in a game. Cursor sessions (Route B) miss the 2 ms window.
- **On Hyprland, playback sends every relative move, covered or not, then the position** (W01 decision 15). Hyprland's locked pointer drifts (by a few px in most runs), so `cursorpos` changes while a game holds the pointer and would cover the game's moves; nothing tells a lock from outside. Hyprland ignores absolute moves while a lock holds, so a locked game gets all its moves and no position; on the desktop the position follows each move within 5 ms. This changes R03 decision 8's skip rule on Hyprland only.
- **W4's bound on Hyprland is 10 ms,** for a position stamped when its answer is read: the layout's publish trailed a position on the new screens by 3.12 ms at most.

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

In the one loaded change where D-Bus ran 200 ms late, xdg-output's notice came first and positions taken on the old screens kept arriving for 200 ms after it. So no window measured on either side of xdg-output's notice bounds where a position belongs: how late D-Bus runs under load has no fixed limit. In every change, including that one, KWin's own layout report reached the daemon before its first position on the new screens (0.04 to 1.4 ms before), in order on one D-Bus connection. W01b therefore has the position script report the layout on every change of the screens, and judges each position by the last layout the script reported before it (W4's "a source whose positions carry their layout"). No timing constant is kept. A change that moves the pointer keeps that order too. With W01b's build on the laptop (eDP-1 at 1.25 beside DP-1 at 1, 3440x1440) and the pointer parked on DP-1, moving DP-1 down by 100 px and back, and disabling DP-1, each had the script report the new layout before the pointer KWin moved with it (0.3 to 1.2 ms before), and enabling DP-1 again reported the layout. xdg-output's notice came 2 to 4 ms after the script's report on a move, and 3.1 to 3.2 s after it on a disable or an enable, so a layout taken from xdg-output alone would have judged the moved pointer by the old screens for seconds. Read from KWin 6.8.2's source, a restarted KWin creates its xdg-output global (`wayland_server.cpp:442`, in `WaylandServer::init`, `main_wayland.cpp:656`) before it claims `org.kde.KWin` (`workspace.cpp:168`, `dbusinterface.cpp:47`, reached from `a.start()` at `main_wayland.cpp:711`), so a connection made on hearing the new name finds it. Measured with W01b's build: after `kwin_wayland_wrapper` was sent `SIGHUP` twice (`pkill -HUP -f '^/usr/bin/kwin_wayland_wrapper'`; `pkill -x` matches nothing, since the process name is cut to 15 characters), `kwin_wayland` ran under a new pid each time while the wrapper kept its pid and the socket; the daemon's connection read `Lost` at once, and the layout came back whole 3.4 s after the new KWin claimed `org.kde.KWin` (which it did 4 to 9 s after the old one left). A new KWin does not answer D-Bus at first: the position script's load into it timed out (2 s) every time. On main the layout script's load took that timeout and the position load, sent after it, succeeded; with the layout script gone, a recording that spans a restart kept no positions after it until W01b tried a failed position load again on the back-off. With that, the script's first report came 10.2 s after the old KWin left, against 9.6 s on main's build, and a recording started after the restart in the same daemon kept its positions. Raw logs: `.claude/local/w01b-spike/` in the main checkout, not committed.

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
