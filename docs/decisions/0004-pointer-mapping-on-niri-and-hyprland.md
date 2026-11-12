# 0004: The screen layout and the pointer on niri and Hyprland

- Status: accepted, 2026-11-12. niri and Plasma measured; Hyprland read from source, to be measured by W01c. W01a added niri at a negative position (one output); two outputs wait for a second monitor.
- Issue: W01 (#75). It shapes the slices in the [W01 brief](../planning/features/w01-other-desktops.md).
- Measured on: the dev box, niri 26.04 (8ed0da4), Linux 7.3.3-1-cachyos, one output (HDMI-A-1, LG 34GN850, 3440x1440 at 85 Hz); the laptop, KWin 6.8.2, eDP-1 (2560x1600) and DP-1 (3440x1440). Read from source: niri v26.04 (6a0a862) with its pinned smithay (ff5fa7d), Hyprland v0.57.2, KWin v6.8.2.

## Decision

- **niri maps the absolute device over the bounding box of all outputs,** as KWin does, so decision 0001's formula holds there, with W and H the box's size. At a fractional scale where physical/scale is not a whole number, niri's box (ceil) can be one logical px larger than the size xdg-output reports (round), so a point lands up to 1 logical px off near the far edges. Accepted (W01 decision 8): no workaround.
- **xdg-output is the layout source on every desktop.** niri and Hyprland report each output's logical position and size there; on Plasma it matched KWin's own layout in every layout tried, so Plasma moves to it too (W01b). The daemon moves the bounding box to (0, 0).
- **niri gives no pointer position in 26.04.** Recordings there keep relative movement. ext-image-copy-capture cursor sessions, on niri's main branch, are the way to get it later (W02).
- **Hyprland, from its source,** maps like KWin and reports sizes consistent with its mapping; it offers the pointer through `cursorpos` and through cursor sessions. W01c measures both before W01d builds one.

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

## Hyprland, from source (v0.57.2; W01c measures)

- **Mapping.** An absolute pointer maps over the bounding box of every monitor's logical box (`src/pointer/PointerManager.cpp:941-956`) unless the user binds the device to an output. A monitor's logical size is round(physical/scale) (`src/output/Monitor.cpp:644`), the value xdg-output sends (`src/protocols/XDGOutput.cpp:127`). Hyprland refuses scales that leave fractional logical pixels (`Monitor.cpp:944`).
- **`cursorpos`.** It answers with floored whole logical px in global coordinates (`src/ipc/s1/Commands.cpp:1384`), over `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`, one connection per request.
- **Cursor sessions.** The position is in fractional logical px from the output's corner (`src/protocols/ImageCopyCapture.cpp:333-357`). It is sent on each monitor commit, so at most once per frame. A cursor move schedules a frame except under VRR fullscreen with `cursor:no_break_fs_vrr` (`Monitor.cpp:1109-1121`, `PointerManager.cpp:476`). It is allowed unless `ecosystem:enforce_permissions` is on (default off, `src/config/values/ConfigValues.cpp:731`), in which case Hyprland asks the user.

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

## The Wayland client's cost

A prototype ran on tokio's current-thread runtime, with wayland-client 0.31.15 and wayland-protocols 0.32.13 (the versions already in `Cargo.lock`; pure Rust, no libwayland) and the workspace's release profile. It bound every `wl_output` and its xdg-output and slept on the connection with `AsyncFd`. Against the same runtime without it, over 60 s idle on niri 26.04:

| | Binary | Resident | CPU ticks | Context switches | Threads |
| --- | --- | --- | --- | --- | --- |
| tokio alone | 443 KB | 2,700 KiB | 0 | 0 | 1 |
| with the xdg-output watcher | 645 KB | 2,940 KiB | 0 | 0 | 1 |

It reported the new layout at once when the scale changed to 1.5 and back. xdg-output v3 sends its `done` on `wl_output`, not on its own object.
