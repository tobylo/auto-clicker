# 0005: An on-screen display above a fullscreen game

- Status: accepted, 2026-11-22 (O01a, #98). It decides whether O01b (#99) and O01c (#100) go ahead as the [O01 brief](../planning/features/o01-on-screen-display.md) describes.
- Measured on:
  - **The laptop:** Plasma 6.8.3 (KWin 6.8.3, plasma-workspace 6.8.3-1.1), then Hyprland 0.57.2 (11cbc6c, from the CachyOS repositories) as the only session. Xwayland 24.1.15, SDL 3.4.20, Linux 7.3.5-2-cachyos. Intel Iris Xe (Raptor Lake-P) beside an RTX 4080 Laptop GPU; Minecraft ran on the Intel GPU (MangoHud's header). Screens: eDP-1, 2560x1600 at 240 Hz (scale 1.25 on Plasma, 1.6 on Hyprland), and DP-1, 3440x1440 at 144 Hz, scale 1, through a CalDigit TS4 dock.
  - **The dev box:** niri-git 26.04-291 (689c1e9), xwayland-satellite 0.8.3, SDL 3.4.20, Linux 7.3.5-1-cachyos, Radeon RX 9070. One screen: HDMI-A-1, 3440x1440 at 85 Hz, scale 1.
  - **The game:** Minecraft Java 26.3. The launcher crashed at start after updating itself that day (`basic_string::_M_construct null not valid`, from the menu too), so the game was started from its version file without it (`mclaunch.py`, offline, single player).
- Read from source: KWin v6.8.3 (3e528155); niri 689c1e91 with smithay 65aa86fa; Hyprland v0.57.2 (11cbc6c5) with aquamarine 0.16.0. Each result below says whether it was measured or read.
- Tools: `spikes/o01-on-screen-display/` (its README says how to run each). The runs' output is kept under the main checkout's `.claude/local/o01a/results/`, with two exceptions. Plasma's single-screen runs were read off the console and are summed up here. The laptop's own files (the Minecraft cycle logs and frame logs, the pattern screenshot, the nested KWin's and the lock run's logs) went with the laptop's reboot; their numbers survive only as written here.

## Decision

- **Go on Plasma, niri and Hyprland.** On all three, a fullscreen game keeps its keyboard focus, its keys, its clicks at the display's place and its locked pointer while the countdown and the badge appear, change and go. This held natively and under XWayland for test windows on all three, and for Minecraft played by hand on all three under XWayland and on Plasma and niri natively; the native Minecraft round on Hyprland has no frame log and no report on input. Where frames were logged, the frame rate did not change at the settings measured. No desktop is refused by name (O01 decision 8): each offers `zwlr_layer_shell_v1` version 5 to an ordinary program.
- **The invariants O1 to O8 and the defaults stand.** The badge cost Minecraft no frames that could be measured, at vsync on and a 120 fps cap. Two costs the brief did not name, both from source: while any surface of ours shows on a screen, the game on that screen is no longer scanned out directly, and Hyprland stops tearing there (point 6). Given both, the owner kept the badges on by default (2026-11-22, O01 decision 2).
- **The countdown is drawn with one sample per pixel, or by filling its rectangles.** The digits and their outline lie on whole pixels, so one sample is exact. Drawn the tray icon's way (16 samples per pixel), the largest common screens take 41 to 257 ms per draw of "88" on the daemon's one thread; one sample takes 6 to 29 ms, and 10 to 50 ms for "99m", the widest label (point 11).
- **Buffers:** the size is round(logical × scale / 120) from `wp_fractional_scale_v1`, shown at the logical size through `wp_viewporter`. This was sharp at 1.25 on Plasma and niri. Shared memory is an anonymous file from `memfd_create` (rustix with `fs`, already in the daemon), written with `pwrite`: no `mmap`, no `unsafe` and no crate beyond `wayland-protocols-wlr`. The daemon grows by 148,912 bytes.
- **A possible flaw on Hyprland, not confirmed:** an X11 test window that holds the pointer by warping it counted an extra motion of 2 to 27 px when a surface of ours appeared, changed its buffer or went. The window's counting then could count moves twice when several queued before its warp landed, so a short stall at those moments would look the same. The counting is fixed, and the effect is to be measured again on Hyprland before anything is reported upstream. Minecraft, which reads raw motion, showed nothing. The owner kept the display available on Hyprland either way (2026-11-22, O01 decision 8).

## How it was measured

The prototype (`o01-probe`) has three modes:
- **`show`:** one layer-shell surface per screen and picture, on the overlay layer, with no keyboard interactivity, an empty input region and exclusive zone −1, drawn into shared memory. It follows screens added and removed and takes commands on stdin: `number N`, `badge`, `both N`, `none`.
- **`window locked|free`:** a fullscreen xdg toplevel that asks for a persistent pointer lock (or none) and prints its focus, keys, buttons, lock events and summed relative motion.
- **`globals` and `outputs`:** list what the compositor offers.

The pictures:
- **The countdown:** the digits alone in indigo with a thin white outline. Every other pixel is fully transparent: no tile and no backdrop (the owner's decision of 2026-11-22). Its surface is 25 % of the screen's height for the characters, plus a quarter of that as padding.
- **The badge:** 40x40 logical, top left, 24 px from the edges.

The first Plasma runs and the first niri runs drew the countdown on a see-through tile. Every run from the Plasma two-screen runs on (Hyprland, Minecraft on every desktop, and niri's native runs repeated) used the transparent countdown, with the same results. The source reading found no compositor deciding focus, picking or direct scanout from a surface's pixels; that transparency changes none of them is an inference from that.

`run.py` drives each case:
- **Devices:** a virtual mouse (relative and absolute axes, two buttons) and a keyboard with one harmless key, right Shift, on `/dev/uinput`.
- **Placement:** the pointer goes to the picture's place (the screen's centre for the countdown, the badge's centre) before the window opens, so a surface appears under the locked pointer.
- **A locked case:** 3 s of relative moves at 1 kHz, a Shift tap every 250 ms and a left click every 500 ms. The display process starts 0.3 s in, shows 3, then 2, then 1, and removes it at 2.0 s, all while input flows. `appear`, `swap` and `remove` split that into one change each.
- **A free case:** three clicks at the picture's place with it shown, and three after it went.
- **Controls:** every case has a control with no display.
- **The guard:** injection stops at the first line saying focus or the lock was lost, and every device is released and destroyed on the way out.

The test windows:
- **Native:** the probe's window, and `sdlwin.py` (SDL 3 in relative mouse mode, Wayland driver).
- **XWayland:** `xwin.py`, an X11 window that grabs the pointer, hides the cursor and warps it back to its centre, as older and Wine games do. Xwayland turns that into a pointer lock. SDL 3's X11 relative mode does not stand in for a game here: on Plasma its motion stopped at the screen's edge with or without a surface (sum 1,279 px).

For Minecraft, the owner played (walking and looking around), fullscreen on DP-1 (laptop) or HDMI-A-1. `cycle.py` showed nothing for 15 s, then did four rounds of: the countdown 3, 2, 1 under the crosshair, 3 s of nothing, the badge for 8 s, 4 s of nothing. MangoHud 0.8.4 logged every frame, and `frames.py` cut the log into those phases, leaving out the first second of each.

## Results

### 1 and 2: offered, and drawn above a fullscreen window on each screen (measured)

| Desktop | Layer-shell | Also offered | Each screen |
| --- | --- | --- | --- |
| Plasma | v5 | viewporter, fractional-scale, tearing-control, presentation | one surface per screen entered each of eDP-1 and DP-1; the owner saw both pictures on both screens above Minecraft |
| niri | v5 | viewporter, fractional-scale, presentation, ext-session-lock | `niri msg layers` listed the surfaces under "Overlay layer" while Minecraft was the focused window; the owner saw both pictures above it |
| Hyprland | v5 | viewporter, fractional-scale, tearing-control, presentation, ext-session-lock | entered eDP-1 and DP-1; the owner saw both above Minecraft |

### 3 to 5: focus, keys, clicks and the locked pointer (measured)

Each cell gives the runs, and in every one the window kept keyboard focus and its lock, and got all 12 key taps and 6 clicks.

| Desktop | Window | Countdown under the pointer (appear, change, go) | Badge | Controls |
| --- | --- | --- | --- | --- |
| Plasma | native, locked | 19 runs, 2,995 to 3,000 moves of 3,000; in the 5 runs with one-way moves the sum was exact (3,001) | 4 runs, 3,000 | 6 runs, 3,000 |
| Plasma | SDL Wayland, locked | 2 runs, 2,995 to 2,996, sum exact | 2 runs, 2,996, sum exact | 2 runs, 3,000 |
| Plasma | X11 (xwin), locked | one screen: 3 runs, 2,995, sum 5,990 X px against the controls' 5,993 to 5,997. Two screens, on DP-1: 1 run, sum 4,248 as its control; on eDP-1: 1 run, 2,962 moves, sum 5,116, with no control in that layout | 3 runs, the same as the controls but for a start jump | 4 runs on one screen, 1 on two |
| niri | native, locked | 8 runs, 3,000, sum exact | 1 run, 3,000 (and 1 with both) | 3 runs, 3,000 |
| niri | X11 (satellite), locked | 5 runs, 2,999, sums 2,999 to 3,067, as the controls' 3,012 to 3,061; 1 more run void (the harness took the window out of fullscreen) | 2 runs | 2 runs |
| Hyprland | native, locked | 5 runs, 3,000, sum exact | 1 run, 3,000 | 2 runs, 3,000 |
| Hyprland | X11 (xwin), locked | 9 runs, 2,999; one extra motion of 2 to 27 px (below) | 2 runs, the same | 7 runs, 2,999, sum 4,557 every time |

- **The X11 window's badge runs** did not put the badge under the pointer: the window warps the pointer back to its own centre, so there the badge appeared in its corner, away from the pointer. Its countdown runs did have the number under the pointer, at the centre.
- **Plasma merges moves, and loses none.** The count drops by 1 to 4 when the countdown appears: 2,998 in 3 of 3 `appear` runs, and 3,000 in 3 of 3 `swap` and 3 of 3 `remove` runs. The badge's appearance, elsewhere and much smaller, never dropped one; which of the two matters was not separated. The sum is unchanged. In source, KWin's libinput thread adds consecutive motion events into one when its main thread drains the queue late (`src/backends/libinput/connection.cpp:373-392`). Mapping and first painting the new surface is that late drain (an inference). A game gets the same total motion.
- **Clicks at the picture's place**, in a window not holding the pointer, reached the window on every desktop: natively and under X11, with the countdown, with the badge, and after either went (on Hyprland, the X11 window's free runs got 6 of 6 clicks at both places). No pointer leave or enter came when a surface appeared or went (measured). In source, an empty input region is never picked (KWin `src/wayland/surface.cpp:820-822`; smithay `src/wayland/compositor/mod.rs:296-308`; Hyprland `src/desktop/state/ViewHitTester.cpp:350-351`). Keyboard interactivity "none" never takes focus (KWin `src/wayland/layershell_v1.cpp:244-246`; niri `src/niri.rs:1384-1411`; Hyprland `src/desktop/view/LayerSurface.cpp:211-226`).
- **Two screens on Plasma:** with the window on DP-1 or on eDP-1 and the display on both, the results matched one screen's.
- **Minecraft, played by hand:** on Plasma (native and XWayland), niri (native and XWayland) and Hyprland under XWayland, the owner saw no lost mouse, no lost keys and no stutter while the pictures appeared and went. After Hyprland's native round the owner reported only the screen it ran on, and no problem.

**Hyprland and the X11 pointer hold (measured, not confirmed).** The X11 window's motion rate stayed at exactly 1 px per move. Matched to the display's nearest event in the three traced runs, extra motion along the direction of travel came:
- when the countdown appeared (27 px, in one run of two);
- when its buffer changed to 2 and to 1 (5 and 2 px, in the other run);
- when it went (21 and 11 px, both runs);
- when the badge went (20 px); the badge's appearance gave none.

The 7 controls never showed it.

**Why it is not confirmed.** In these runs the window counted each motion from the screen's centre. Moves that queue before its warp back to the centre lands are then counted from the centre each time: two queued moves of 1 px count as 3. A stall of a few milliseconds when a surface maps, commits or unmaps would therefore show as extra motion, without Hyprland sending any. The window now counts from the last position it saw and skips the motion of its own warps; rerun on niri, it counted 2,999 px for 2,999 moves, with and without the countdown. It was not rerun on Hyprland: the laptop was off the network by then.

From source:
- Mapping a layer surface makes Hyprland check the pointer again (`refocusPointer`, `src/desktop/view/LayerSurface.cpp:230`; `src/managers/input/InputManager.cpp:1846-1851`). Unmapping sends a simulated pointer movement (`simulateMouseMovement`, `LayerSurface.cpp:311`; `InputManager.cpp:195-198`).
- A commit calls neither for a surface without pointer focus (`LayerSurface.cpp:370-371`), so the extra motion at buffer changes is not traced to a line.
- Hyprland's locked pointer drifts with the moves (decision 0004), so these re-checks likely hand Xwayland the drifted position, which the warping client reads as motion (an inference).

What it means for games:
- A native Wayland game on Hyprland gets nothing extra.
- If the effect is Hyprland's, an X11 game that reads warped motion sees its view nudged when the display appears, changes or goes; if it is the window's counting, nothing happens.
- Minecraft under XWayland reads raw motion (SDL 3's XInput2): the owner saw no jump.

### 6: what it costs the game (measured, with the scanout rules read from source)

Minecraft 26.3 at the owner's settings, vsync on and a 120 fps cap, so the game's own limiter sets the median on the laptop's 144 Hz screen:

| Desktop | Window | Nothing shown | Countdown | Badge |
| --- | --- | --- | --- | --- |
| Plasma, DP-1 at 144 Hz | native | 117.2 fps; median 8.28 ms, p99 14.04 | 118.4 fps; 8.31, 13.50 | 117.6 fps; 8.32, 12.53 |
| Plasma, DP-1 | XWayland | 116.0 fps; 8.34, 16.26 | 118.0 fps; 8.34, 16.53 | 119.1 fps; 8.34, 12.74 |
| Hyprland, DP-1 | XWayland | 120.0 fps; 8.34, 11.58 | 120.0 fps; 8.35, 11.63 | 119.8 fps; 8.35, 10.65 |
| niri, HDMI-A-1 at 85 Hz | native | 85.0 fps; 11.76, 12.18 | 85.0 fps; 11.76, 12.15 | 85.0 fps; 11.76, 12.24 |
| niri | XWayland | 85.0 fps; 11.76, 12.28 | 85.0 fps; 11.75, 14.27 | 84.8 fps; 11.75, 13.99 |

- Each row is one round of about 90 s. MangoHud's log names its start to the second, so a frame's time is known to within a second, and each phase's edges are left out: on niri the first and last second (about 2,100 frames with nothing shown, 340 with the countdown and 2,040 with the badge). The laptop's rows leave out only the first second (3,000 to 4,000, 700 to 950 and 2,400 to 3,400 frames), since their logs were lost before the last second could be cut too. No difference beyond one round's spread.
- What this does not show: a game running uncapped, where composition could take frames, and KWin on amdgpu or NVIDIA. On the laptop the game rendered on the Intel GPU, the one case where KWin may keep the game on a plane of its own (below).
- Hyprland's native round has no frame log: the owner's answer came after the 4-minute log had ended.
- Plasma's native round overlapped another session's test: from 15:09:30 to 15:10:10 a test build of the daemon and its settings window ran on DP-1, the window above Minecraft. That covers about the last 30 s of that round's 90 s. Its frame log went with the laptop's reboot, so those seconds could not be cut out afterwards. Plasma's XWayland round, on the same screen with nothing else running, gave the same result, and the Plasma result rests on it.
- MangoHud's `no_display` option also turns its logging off, so these rounds show a small FPS counter in a corner, the same in every phase. Earlier rounds without logs are left out.

**Direct scanout (source, and `hyprctl` on Hyprland):** while any surface of ours shows on a screen, the badge included, every frame of the game on that screen is composited, except possibly on Intel GPUs, where KWin's release builds may keep the game on a plane of its own (an inference, below).
- KWin puts only dmabuf surfaces on planes (`src/scene/workspacescene.cpp:265`). It uses extra planes in release builds only on Intel (`src/backends/drm/drm_output.cpp:361-372`), so on amdgpu and NVIDIA the game loses direct scanout.
- niri leaves overlay planes off by default (`niri-config/src/lib.rs:2859`), and smithay scans out only the topmost element (`src/backend/drm/compositor/mod.rs:1995`).
- Hyprland blocks a solitary client while any overlay-layer surface is visible (`src/output/Monitor.cpp:1774-1780`). `hyprctl monitors` showed `solitaryBlockedBy: other overlays` during the badge. Its direct scanout is off by default anyway ("user settings").

**VRR (source):** kept on all three. KWin's automatic VRR follows the active fullscreen window (`src/compositor.cpp:881-882`). niri leaves overlay surfaces out on purpose (`src/niri.rs:5196-5197`). Hyprland follows the fullscreen workspace (`src/config/shared/monitor/MonitorRuleManager.cpp:256-258`). VRR was off on every screen measured (Plasma "Never", niri "disabled", Hyprland `misc:vrr` 0), so it was not measured on.

**Tearing (source):**
- **KWin:** kept. Tearing does not depend on direct scanout (`src/compositor.cpp:883`), and our surface is composited onto the primary plane, which does not downgrade async flips (`drm_output.cpp:307-314`; an inference).
- **niri:** does not tear (no `wp_tearing_control`).
- **Hyprland:** stops tearing while any surface of ours shows (`Monitor.cpp:1871-1874`), so a user with `allow_tearing` loses tearing for as long as the badge stays.
- Tearing was off on both desktops that have it, so it was not measured.

### 7: screens added and removed, and KWin restarted (measured)

- **niri** (`niri msg output HDMI-A-1 off`, then `on`): the display's own connection heard `closed` for both surfaces, then the output's global removed. On `on` it heard a new global and made both surfaces again from its own registry.
- **Hyprland** (`hyprctl output create headless`, then `remove`): the new output got both surfaces. On removal, `closed` came for both before the global went.
- **KWin restarted:** a nested `kwin_wayland_wrapper --virtual` with the display on it; its `kwin_wayland` was killed. The display's connection ended at once ("Broken pipe"). The wrapper's new KWin answered on the same socket within 3 s, and a new connection listed the output.
- **A screen removed on Plasma** was not measured live: the dock dropped in a loop during the session, never while the display showed. From source, KWin sends `closed` on `outputRemoved` and destroys the surface (`src/layershellv1window.cpp:58-59,329-334`), and a surface made for a screen already gone is closed at once (`src/layershellv1integration.cpp:35-38`).

So the display hears screens come and go from its own connection on all three, and after a lost connection the layout's signal is the only one left (O3).

### 8: the lock screen and sleep

- **Plasma (measured):** with both pictures shown, `loginctl lock-session` locked the screen. The owner saw neither picture on the lock screen, and both were back after unlocking. In source, while locked KWin draws only lock-screen, input-method and lock-screen-overlay windows (`src/scene/windowitem.cpp:166-167`).
- **niri and Hyprland (source):** while locked, niri draws only the lock surface and a background (`src/niri.rs:4704-4728`). Hyprland draws no layers unless a layer rule puts them above the lock (`src/render/Renderer.cpp:1211-1215,1781-1812`). Hyprland had no locker installed to measure.
- **Sleep (not measured):** the laptop crashed going to sleep. Its journal ends at `PM: suspend entry (deep)`; it was the first sleep logged on that laptop, and it happened with the Thunderbolt dock attached. Below anything a Wayland client can affect, and not retried. Suspending the dev box would have stopped this session. O7 already removes the display when the session locks or the system prepares to sleep, by changing the view.

### 9: niri's cursor sessions beside the display (measured)

W02's `hm replay 300 0 3440 1440`, which puts the pointer at 300 random places and compares each with the cursor session's position: 300 of 300 exact (0 px on either axis) without the display, and 300 of 300 exact with the countdown and the badge shown.

### 10: sharpness and the buffer's size (measured)

- **Plasma at 1.25:** one-pixel stripes, 400x200 logical, drawn into a 500x250 buffer at the preferred fractional scale 150/120, with viewport destination 400x200. In a screenshot at native resolution the stripes alternated 0 and 255 with no grey pixel among 110,400.
- **niri at 1.25:** the same, also 0 of 110,400.
- **Hyprland at 1.6:** a 640x320 buffer for 400x200 logical, at 192/120, a fractional scale whose buffer sides come out whole. Sharpness not measured: no capture tool was installed.
- **Before configure:** KWin also sends `preferred_buffer_scale` 2 (whole numbers only). The fractional scale is what to use. All three send the fractional scale before or right after `configure`.
- **The rule for O01b:** buffer = round(logical × scale / 120) per side, checked arithmetic, viewport destination = logical size. Until the scale arrives, draw at 120 and draw again when it does.

### 11: what the display costs the daemon (measured)

- **Memory:** the `show` process idled at 3.0 MiB connected with nothing shown, 3.2 MiB with the badge, and 4.2 MiB after a countdown. The image buffers are kept by the allocator; O01b frees or reuses them.
- **Wakeups (Plasma):** the badge, shown for 60 s, woke the process 0 times and used 0 CPU ticks. A 10 s countdown with the badge woke it 33 times (about 3 a second: the command, and the compositor's answers to a new buffer on each screen) and used 14 ticks, which is the drawing.
- **One draw of the countdown** (release build, `o01-probe draw W H SCALE [SECONDS]`), as "88" and as "99m", the widest label (about 1.5 times as wide):

| Screen | "88": buffer | "88", 16 samples a pixel (the tray's way): dev box / laptop | "88", 1 sample: dev box / laptop | "99m": buffer, 1 sample, dev box |
| --- | --- | --- | --- | --- |
| 3440x1440 at 1 | 619x540 | 41.3 / 63.5 ms | 5.7 / 6.1 ms | 914x540, 10.1 ms |
| 2048x1280 at 1.25 | 688x600 | 49.9 / 78.8 ms | 6.9 ms / – | 1016x600, 11.5 ms |
| 3840x2160 at 1 | 929x810 | 93.0 / 143.5 ms | 12.3 / 14.0 ms | 1372x810, 21.2 ms |
| 3840x2160 at 1.5 | 1394x1215 | 204.6 ms / – | 28.9 ms / – | 2058x1215, 50.1 ms |
| 5120x2880 at 1 | 1238x1080 | 164.6 / 256.7 ms | 23.3 / 25.7 ms | 1829x1080, 37.8 ms |

  Every screen is drawn in turn on the daemon's one thread. One sample a pixel is exact for whole-pixel rectangles. Filling the rectangles row by row would cost about a memset per buffer (not measured). The badge's rounded tile keeps 16 samples: 40x40 to 80x80 px take 0.04 to 0.4 ms.
- **Binary size:** a copy of main (fb75789) with `wayland-protocols-wlr` 0.3.12 and rustix (`std`, `fs`) as dependencies of `backend-wayland`, and a layer surface with a viewport, a fractional scale and an shm buffer actually made on a path the daemon reaches. Release daemon: 6,396,392 bytes before, 6,545,304 bytes after, so +148,912 bytes. Cargo.lock gained no new package, only the two dependency lines in `backend-wayland`'s entry.
- **Shared memory:** `memfd_create` (rustix `fs`, already a dependency of `backend-kwin`), `set_len`, `write_all_at`, then seals against shrinking and growing. Neither `mmap` nor `unsafe` is needed, and no crate beyond `wayland-protocols-wlr`.

## For O01b and O01c

- **Draw the countdown by its rectangles:** one sample a pixel, or filled rows. Not 16 samples.
- **Buffers:** round(logical × fractional scale / 120) and a viewport, as in point 10.
- **Screens:** heard from the display's own connection (`wl_output` globals added and removed, and `closed`). After a lost connection, the layout's signal (O3).
- **The guide** names the namespace and how to keep the display out of a recording or stream: niri's `layer-rule` with `block-out-from "screencast"`, or `"screen-capture"`, matched on the namespace (`src/layer/mod.rs:98-103`); Hyprland's layer rule `no_screen_share` (`src/desktop/rule/layerRule/LayerRule.cpp:64`). KWin has no rule for layer surfaces: window rules do not apply to them (`src/window.cpp:4276-4278`).
- **The guide** also says: while the badge shows, the game on that screen is composited (no direct scanout), and on Hyprland it does not tear.

## For an upstream report to Hyprland (held until measured again)

Not to be filed until `xwin.py`, with its fixed counting, shows the extra motion again on Hyprland, with controls. If it does, the facts are these.

- **Reproducer:** Hyprland 0.57.2. A fullscreen X11 client under Xwayland grabs the pointer, hides its cursor and warps the pointer back to its centre after each move (`xwin.py locked`). Relative moves come from a mouse. A layer-shell client maps, or unmaps, an overlay-layer surface with an empty input region anywhere on the screens.
- **Observed (with the old counting):** when the layer surface maps, changes its buffer or unmaps, the X11 client receives an extra motion of 2 to 27 px along the direction of travel. In 7 controls without the layer surface, the summed motion was identical every time (4,557 px).
- **Expected:** a surface that takes no input changes nothing for the client that holds the pointer.
- **Where:** `CLayerSurface::onMap` calls `refocusPointer()` (`src/desktop/view/LayerSurface.cpp:230`) and `onUnmap` calls `simulateMouseMovement()` (`:311`, `src/managers/input/InputManager.cpp:195-198`). With the pointer locked and drifting (decision 0004), these likely send the drifted position (an inference). The path for a buffer change was not found.
- **Where it hurts:** X11 games that read warped motion (older and Wine games) see their view nudged when any layer surface appears or goes: notifications, bars and this display.
- **Already reported?** No match on 2026-11-22 in Hyprland's issues or discussions (searched: simulateMouseMovement, refocusPointer, layer surface pointer jump, locked pointer layer surface, Xwayland warp pointer jump, mouse jumps when notification). Related, not the same: discussion #15565 (since 0.56, X11 games sometimes get a pointer warp as one large jump when they grab the pointer; no layer surface involved).
