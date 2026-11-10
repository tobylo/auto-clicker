# 0001: Reading the cursor position on KWin

- Status: accepted, 2026-11-10
- Issue: X01 (#7). It decides R03 (#16) and U03 (#28).
- Tested on: Plasma 6.8.2 (KWin 6.8.2, plasma-workspace 6.8.2), Linux 7.3.3-1-cachyos, libinput 1.32.0, Qt 6.12.0, xdg-desktop-portal 1.22.1 with xdg-desktop-portal-kde 6.8.2, libei 1.6.0. The machine is the target laptop (ASUS M16, i9-13900H). Its panel, eDP-1, is 2560x1600 at scale 1.25 (2048x1280 logical). An external DP-1 is 3440x1440 at scale 1, and at 1.5 for one run.
- Code: [spikes/x01-cursor-position/](../../spikes/x01-cursor-position/README.md).

## Decision

**Recordings can carry absolute positions that replay to within a pixel of where the real pointer was.** Map D3 takes its first branch.

- **Reading the position.** While a recording runs, the daemon loads a small KWin script through `org.kde.KWin /Scripting`. On every `workspace.cursorPosChanged` the script sends `workspace.cursorPos` to a D-Bus name the daemon owns, and it reports the layout size whenever the layout changes. The daemon unloads the script when the recording stops.
- **Replaying it.** Playback moves C01's absolute axes to the recorded position with the formula below. On every layout tested it lands within 0.65 physical px of where the real pointer was. Almost all of that is the script's rounding to whole logical px; the formula itself adds under 0.05 px.

### The mapping formula

KWin hands an absolute pointer event to libinput with the size of the bounding box of all outputs, `workspace()->geometry().size()` (`src/backends/libinput/connection.cpp:394`). That size is in logical px. libinput scales by `(v - min) * size / (max - min + 1)` (`absinfo_scale_axis`, `src/util-input-event.h:93`, range from `absinfo_range` at line 73). The device's range is 0..=65535, so a value `v` lands at `v * W / 65536`. The inverse, for a layout position `x` in logical px with `W` the layout width:

```text
v = clamp(round(x * 65536 / W), 0, 65535)      and likewise y with H
```

- **Origin.** Positions are measured from the bounding box's top-left. KWin ignores the box's origin, and KScreen always normalises the layout to start at (0,0). Placing DP-1 at x = -3440 came back as 2048,0.
- **Device.** C01 puts the absolute axes on the same virtual mouse as the relative axes and the buttons (`crates/backend-kernel/src/devices.rs`, `configure_mouse`), not on a separate device. The checks above used that device.
- **Size.** `W` and `H` are KWin's own layout size, `workspace.virtualScreenSize`, which the script reports. Do not take them from KScreen: at scale 1.5, `kscreen-doctor` lists DP-1 as 2294 wide, while KWin's layout is 2293 wide (4341 in all). One pixel of difference in `W` moves a point near the right edge by almost a whole pixel.
- **Rounding.** Rounding `v` costs at most `W / 131072` px: 0.04 px at W = 5488.

## Measurements

Every number below came from the commands under "Commands", run on the committed spike code on 2026-11-10: latency, pointer lock, lifecycle and the dual-output mapping between 02:49 and 02:54 (after the laptop had restarted), CPU and the other layouts between 01:01 and 01:08. Earlier runs of the same checks gave the same results. The laptop was otherwise idle and its mouse and keyboard untouched, and all input was synthetic, from C01's virtual mouse, except under "Real mouse".

### Latency: uinput write to receipt

Absolute moves went to distinct positions, paced on absolute deadlines. The time is taken just before each move is written, so every figure is an upper bound. Each call was matched to its move. Three sweeps of 900 moves were run per rate, 2700 moves in all.

| Rate | Moves | Callbacks | Received | p50 | p90 | p99 | max |
|---|---|---|---|---|---|---|---|
| 125 Hz | 2700 | 2700 | 2700 | 0.55 ms | 0.71 ms | 0.81 ms | 1.07 ms |
| 500 Hz | 2700 | 2700 | 2700 | 0.42 ms | 0.58 ms | 0.69 ms | 0.97 ms |
| 1000 Hz | 2700 | 2701 | 2701 | 0.26 ms | 0.38 ms | 0.54 ms | 1.19 ms |
| 2000 Hz | 2700 | 2700 | 2700 | 0.20 ms | 0.28 ms | 0.38 ms | 1.04 ms |

- **Nothing coalesces or drops.** KWin calls the script once per pointer move, and every call arrives. The extra callback at 1000 Hz is the move onto the sweep's start row.
- **The script's own time is too coarse to use.** It has only `Date.now()`, wall-clock time in whole milliseconds. Script-to-receipt read between −0.33 and +0.54 ms at p50 across runs, an error of up to a millisecond either way. The end-to-end figure above is measured on one µs clock and bounds it.
- **The 1000 and 2000 Hz rows are synthetic.** "Real mouse" below checks the rate a physical mouse produces.

### CPU

Each run is 10 s of relative moves at the given rate. Figures are % of one core, as two runs. The baseline ("script off") includes KWin drawing the moving cursor on a 240 Hz panel.

| Rate | Script off: KWin | Script on: KWin | dbus-broker | Receiver (Python) | Script loaded, receiver dead: KWin |
|---|---|---|---|---|---|
| idle | 0.0 / 0.2 | 0.1 / 0.1 | 0.0 | 0.0 | 0.0 / 0.0 |
| 125 Hz | 5.0 / 7.1 | 6.4 / 9.2 | 0.4–0.6 | 1.2–1.3 | 6.1 / 8.8 |
| 1000 Hz | 13.7 / 16.7 | 20.2 / 24.1 | 2.1–2.6 | 6.1–6.3 | 19.3 / 22.5 |
| 2000 Hz | 19.1 / 19.4 | 29.5 / 34.4 | 4.9–5.2 | 11.4–11.8 | 29.8 / 35.0 |

- **Idle.** A loaded script costs nothing while the pointer is still. The daemon still loads it only while recording, so a daemon that is not recording has no script in KWin.
- **While moving.** The script adds about 7% of a core to KWin at 1000 Hz and 10–15% at 2000 Hz.
- **Delivery.** Every call the script made arrived. The script was called slightly less often than moves were injected at the higher rates (9999 of 10000 at 1000 Hz, 19992 and 19972 of 20000 at 2000 Hz): a move that leaves the pointer where it was does not fire `Cursor::posChanged` (`src/cursor.cpp:191`). The loss is upstream of the script, not on the bus.
- **A dead receiver costs as much as a live one.** With the script loaded and nobody owning the name, every call still leaves KWin and fails at the bus.
- **The receiver will be cheaper in production.** Its figures are a Python receiver; a Rust one in the daemon will cost less.

### Real mouse

The owner moved a Razer Viper V3 Pro in fast circles over the desktop during four 10 s runs of `cpu --hz 0`, two with the script on and two off.

| Script | Positions reported | KWin | dbus-broker | Receiver |
|---|---|---|---|---|
| on | 10408, 10834 (about 900–1080 per second) | 18.7%, 17.9% | 1.9%, 2.2% | 5.1%, 6.9% |
| off | – | 12.1%, 11.7% | 0.0% | 0.0% |

- **A physical high-rate mouse behaves like the synthetic 1000 Hz stream.** Every position was delivered, and KWin used about 6% more of a core. These counts include the second before and the half second after each 10 s run (the harness has since counted only the timed window), so the rate lies between count / 11.5 s and count / 10 s. A later attempt to re-measure it within the window did not cover the movement and is not used.
- **Latency was not measured here.** It needs the evdev event time, and the user is not in the `input` group.

### Playback: where a replayed position lands

`mapping`, run by `layouts.sh`, checks 19 points per output: a grid plus the four corner pixels. For each point it:

1. moves there;
2. nudges the pointer with relative motion, so that it rests at a sub-pixel position the way a real mouse does;
3. reads that original position and the recorded one (the script's whole px);
4. moves away, replays the recorded position through the formula, and reads where the pointer landed.

Sub-pixel positions come from `org.freedesktop.a11y.PointerLocator.QueryPointer` over a fullscreen probe window on each output. It reads the same KWin cursor as the script, so it adds resolution, not an independent check; it agreed with the script within 0.5 px on every point.

The independent check is the probe window itself. Each GTK 4 probe prints its own surface-local pointer position, which a client like U03's overlay would see.
- **Panel alone:** across 57 reads it matched `QueryPointer` within 0.001 px.
- **Two outputs:** the median was 0.001 px and p90 0.043 px. The remaining reads were taken before the probe had reported a motion event since the pointer entered it (no motion yet, or the last one stale), because the probe prints motion only, not enter events.

Results in physical px, the largest over all points:

| Layout (logical) | Replayed vs recorded | Replayed vs original pointer |
|---|---|---|
| Panel alone, 2048x1280 @1.25 | 0.010 | 0.568 |
| Panel + DP-1 right @1, 5488x1440 | 0.049 | 0.559 |
| DP-1 left of the panel, 5488x1440 | 0.049 | 0.651 |
| DP-1 above the panel, 3440x2720 | 0.031 | 0.568 |
| DP-1 right @1.5, 4341x1280 (fractional logical width) | 0.044 | 0.574 |
| DP-1 placed at x = -3440 (KScreen normalised it to the right) | 0.049 | 0.559 |

- **"Within 1 px" holds in both senses.** The gap between the original and the replayed position is the script's rounding to whole logical px. It is at most 0.5 logical px, which is 0.625 physical px at scale 1.25. The formula itself adds less than 0.05 px.
- **A move to the panel's top-left pixel lands 1 px inside it.** This happened in the four dual-output layouts, not with the panel alone. It is consistent with KWin pushing the cursor back from an active screen corner (`Edge::pushCursorBack`, `src/screenedge.cpp:522`; Plasma's default top-left corner opens Overview). A real mouse would be pushed back the same way.
- **Targets on no output are clamped.** Points inside the bounding box that no output covers (the strip under the shorter panel) land on the nearest output's edge. For example, (686, 1350) lands at (686, 1279).

### Pointer lock

`lock` opens a fullscreen SDL3 window in relative mouse mode, which locks the pointer through Wayland pointer constraints the way games do.

- **While locked:** 200 relative moves gave no positions, and the window received all of them as relative motion.
- **On unlock, with no input:** the pointer moved by (+1, +1), and the script reported that position once. The window had received a (+1, +1) nudge while locked, so this is consistent with KWin warping to the lock's cursor-position hint on unlock (`pointer_input.cpp:756-758`, `processWarp(focus()->mapFromLocal(*hint))`).
- **After unlock:** every move was reported again.

The locked pointer stands still in KWin (`PointerInputRedirection::updatePosition` returns early while `m_locked`), so there is no position to record. Recording relative motion covers it, as map D4 already says.

### Lifecycle

- **Loading needs no consent and no setting.** `loadScript`, `run` and `unloadScript` worked from an unprivileged session client with no prompt. `~/.config/kwinrc` had the same checksum before and after each load (`lifecycle.sh`).
- **`run()` returns after the script has run.** Called over D-Bus, `Script::run` defers its reply until the script has been read and evaluated (`src/scripting/scripting.cpp:166`). The first Layout call arrived about 3 ms after `loadScript`.
- **A crashed receiver leaves the script loaded.** After the receiver was killed with SIGKILL, `isScriptLoaded("x01-cursor")` was true and KWin kept paying for the calls (see CPU). The next start found the script by its plugin name and unloaded it before loading its own.
- **KWin restarts drop the script.** On a nested, virtual KWin in a private D-Bus session, the script was gone after KWin was killed and started again, and `org.kde.KWin` had a new owner. A daemon recording across a KWin restart has to watch that name and load the script again. The live session's KWin was never restarted.

## Other routes

- **KWin's own D-Bus interfaces.** Every object under `org.kde.KWin` was introspected for cursor, pointer, position and mouse members. The only position source is `org.freedesktop.a11y.PointerLocator.QueryPointer` on `/org/freedesktop/a11y/Manager`, new in Plasma 6.8 (first tagged v6.7.90, the 6.8 beta).
  - It has no permission check, and it returns the pointer's sub-pixel position relative to the window under the pointer, with that window's pid.
  - It then sends a single `PointerPositionChanged` signal on the next move, as a throttle.
  - It needs no script, but the position is window-relative. Turning it into a layout position needs the window's geometry, which only a script can supply. It is an a11y interface whose keyboard half is limited to the Orca screen reader, so the same may happen to it.
  - The spike uses it only to read sub-pixel positions over its own fullscreen windows.
- **InputCapture portal.** Version 2 is present (`busctl --user introspect org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.InputCapture`). Pointer barriers report a position only when the pointer crosses a barrier, and then the portal takes the input away from the compositor until it is released. Neither suits passive recording. No session was opened, since that would have needed a consent prompt for a route that cannot serve.
- **Plasma 6.8 APIs.** No other interface exposes the global cursor position. A scripted KWin *effect* has a sub-pixel `effects.cursorPos`, but effects must be installed into KWin's data directory and enabled in its config, which is a setting. Wayland clients see the pointer only over their own surfaces, which is the route U03's overlay takes.

## Limitations

- **Precision is whole logical px.** `workspace.cursorPos` is `Cursor::pos().toPoint()` (`src/scripting/workspace_wrapper.cpp:148`). With the axis rounding, a replay can be off by up to `(0.5 + W / 131072) * scale` physical px per axis: 0.68 px at scale 1.25 with W = 5488 (0.65 measured), but just over 1 px at scale 2. "Within 1 px" therefore holds up to about scale 1.8 to 1.9, depending on the layout width; at scale 2 and above it needs the sub-pixel position upstream report 3 asks for.
- **The script cannot tell which device moved the pointer.** It also reports moves made by the program's own virtual mouse. Recording is refused while clicking or playing (R01), so the two never overlap.
- **Anyone in the session can load KWin scripts.** Any session process can also call the receiver's D-Bus name, so the receiver must accept calls only from the current owner of `org.kde.KWin`.
- **Pointer lock gives no positions.** While a game locks the pointer, recordings fall back to relative motion.
- **The layout must match at playback.** A position is only meaningful against the layout it was recorded on. The size alone cannot tell: DP-1 left of the panel and DP-1 right of it are both 5488x1440, yet (5000, 1350) is on DP-1 in one and below the panel (clamped) in the other. Recordings must carry every output's rectangle and scale, and playback must compare them.
- **Latency was measured on synthetic input only.** A physical mouse was checked for rate, delivery and CPU ("Real mouse").
- **Shake-to-find is irrelevant.** KDE's "shake cursor to find it" effect enlarges the drawn cursor during fast moves. It does not change the position.

## Upstream reports (for the owner)

1. **KWin: `QueryPointer` adds a connection per call.**
   - Each call adds a new single-shot connection to `Cursor::posChanged` (`src/a11ymanager.cpp:203-208` in 6.8.2, unchanged on master at a2a7855509, 2026-11-09).
   - Observed: 100 calls without motion, then one move, gave 100 `PointerPositionChanged` signals. Expected one, since the interface documents the signal as a throttle.
   - Reproducer: `spikes/x01-cursor-position/harness/query_pointer_signals.py`, which calls `QueryPointer` N times, moves the pointer, and counts the signals.
   - Effect: a screen reader that polls while the pointer is still is flooded on the next move. It does not affect this program.
   - Not reported, as of 2026-11-10:
     - KDE Bugzilla quicksearch for "PointerLocator", "QueryPointer" and "PointerPositionChanged" found nothing.
     - KWin's GitLab issues and merge requests, searched for "PointerLocator" and "QueryPointer" through the API, show only the merge request that added the interface (!9825, commit 23b0427864, first released in 6.7.90) and draft !8883, which adds an accessibility bus address and does not touch this.
2. **KWin: `QueryPointer` has no permission check, while its sibling `KeyboardMonitor` is limited to Orca.** Any session client can learn the pointer position and the pid of the window under it. This is worth raising as a question rather than a bug.
3. **Wishlist: a sub-pixel `cursorPos` for KWin scripts.** `workspace.cursorPos` drops the sub-pixel part. A `QPointF` property would remove this program's only rounding error. Low priority on this laptop, where the error stays under 0.7 physical px; it matters at scale 2 and above.

## Commands

On the laptop, from `spikes/x01-cursor-position` (after `(cd inject && cargo build --release --locked)`), with `/tmp/auto-clicker-spike.lock` held:

```sh
for hz in 125 500 1000 2000; do python3 harness/x01.py latency --hz $hz --sweeps 3 --count 900; done
for rep in 1 2; do for hz in 0 125 1000 2000; do for mode in off on dead; do
  python3 harness/x01.py cpu --hz $hz --seconds 10 --mode $mode; done; done; done
for mode in on off on off; do python3 harness/x01.py cpu --hz 0 --seconds 10 --mode $mode; done   # real mouse, moved by hand
./layouts.sh               # the playback table; restores eDP-1 0,0 @1.25 and DP-1 2048,0 @1
python3 harness/x01.py lock
./lifecycle.sh
```

Versions: `kwin_wayland --version`, `uname -r`, `pacman -Q libinput plasma-workspace qt6-base xdg-desktop-portal xdg-desktop-portal-kde libei`.
