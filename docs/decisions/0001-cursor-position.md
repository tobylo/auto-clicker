# 0001: Reading the cursor position on KWin

- Status: accepted, 2026-11-10
- Issue: X01 (#7). It decides R03 (#16) and U03 (#28).
- Tested on: Plasma 6.8.2 (KWin 6.8.2, plasma-workspace 6.8.2), Linux 7.3.3-1-cachyos, libinput 1.32.0, Qt 6.12.0, xdg-desktop-portal 1.22.1 with xdg-desktop-portal-kde 6.8.2, libei 1.6.0. The machine is the target laptop (ASUS M16, i9-13900H). Its panel, eDP-1, is 2560x1600 at scale 1.25 (2048x1280 logical). An external DP-1 is 3440x1440 at scale 1, and at 1.5 for one run.
- Code: [spikes/x01-cursor-position/](../../spikes/x01-cursor-position/README.md).

## Decision

**Recordings can carry exact absolute positions.** Map D3 takes its first branch.

- **Reading the position.** While a recording runs, the daemon loads a small KWin script through `org.kde.KWin /Scripting`. On every `workspace.cursorPosChanged` the script sends `workspace.cursorPos` to a D-Bus name the daemon owns, and it reports the layout size whenever the layout changes. The daemon unloads the script when the recording stops.
- **Replaying it.** Playback moves C01's absolute axes to the recorded position with the formula below. On every layout tested it lands within 0.05 physical px of the recorded position, and within 0.65 physical px of where the real pointer was.

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

Every number below came from the commands under "Commands". The laptop was otherwise idle, and its mouse and keyboard were untouched. All input was synthetic, from C01's virtual mouse.

### Latency: uinput write to receipt

Absolute moves went to distinct positions, paced on absolute deadlines. Each call was matched to its move. Three sweeps of 900 moves were run per rate, 2700 moves in all.

| Rate | Moves | Callbacks | Received | p50 | p90 | p99 | max |
|---|---|---|---|---|---|---|---|
| 125 Hz | 2700 | 2700 | 2700 | 0.40 ms | 0.51 ms | 0.62 ms | 0.88 ms |
| 500 Hz | 2700 | 2700 | 2700 | 0.23 ms | 0.35 ms | 0.46 ms | 0.89 ms |
| 1000 Hz | 2700 | 2700 | 2700 | 0.16 ms | 0.23 ms | 0.37 ms | 0.70 ms |
| 2000 Hz | 2700 | 2700 | 2700 | 0.17 ms | 0.22 ms | 0.32 ms | 0.90 ms |

- **Nothing coalesces or drops.** KWin calls the script once per pointer move, and every call arrives.
- **The script's own time is too coarse to use.** It has only `Date.now()`, whole wall-clock milliseconds rounded down, so script-to-receipt reads 0.15–0.54 ms at p50 with a bias of up to +1 ms. The end-to-end figure above is measured on one µs clock and bounds it.
- **The 1000 and 2000 Hz rows are synthetic.** A real 1000 Hz mouse was not tested.

### CPU

Each run is 10 s of relative moves at the given rate. Figures are % of one core, as two runs. The baseline ("script off") includes KWin drawing the moving cursor on a 240 Hz panel.

| Rate | Script off: KWin | Script on: KWin | dbus-broker | Receiver (Python) | Script loaded, receiver dead: KWin |
|---|---|---|---|---|---|
| idle | 0.0 / 0.3 | 0.0 / 0.0 | 0.0 | 0.0 | 0.1 / 0.1 |
| 125 Hz | 5.8 / 5.8 | 7.1 / 6.8 | 0.4–0.5 | 1.2–1.8 | 7.7 / 7.0 |
| 1000 Hz | 14.1 / 13.5 | 20.7 / 20.7 | 1.6–1.9 | 6.0–7.0 | 20.9 / 13.6 |
| 2000 Hz | 20.2 / 19.2 | 30.3 / 29.8 | 3.8–3.9 | 12.0–13.2 | 31.5 / 32.8 |

- **Idle.** A loaded script costs nothing while the pointer is still. The daemon still loads it only while recording, so a daemon that is not recording has no script in KWin.
- **While moving.** The script adds about 7% of a core to KWin at 1000 Hz and 10% at 2000 Hz.
- **A dead receiver costs as much as a live one.** With the script loaded and nobody owning the name, every call still leaves KWin and fails at the bus. One of the two 1000 Hz runs read like "off", so treat that cell as noisy.
- **The receiver will be cheaper in production.** Its figures are a Python receiver; a Rust one in the daemon will cost less.

### Playback: where a replayed position lands

`mapping`, run by `layouts.sh`, checks 19 points per output: a grid plus the four corner pixels. For each point it:

1. moves there;
2. nudges the pointer with relative motion, so that it rests at a sub-pixel position the way a real mouse does;
3. reads that original position and the recorded one (the script's whole px);
4. moves away, replays the recorded position through the formula, and reads where the pointer landed.

Sub-pixel positions come from `org.freedesktop.a11y.PointerLocator.QueryPointer` over a fullscreen probe window on each output. On every point it agreed with the script within 0.5 px.

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
- **A move to the panel's top-left pixel lands 1 px inside it.** This happened in the four dual-output layouts. It is consistent with KWin pushing the cursor back from an active screen corner (`Edge::pushCursorBack`, `src/screenedge.cpp:522`; Plasma's default top-left corner opens Overview). A real mouse is pushed back the same way.
- **Targets on no output are clamped.** Points inside the bounding box that no output covers (the strip under the shorter panel) land on the nearest output's edge. For example, (686, 1350) lands at (686, 1279).

### Pointer lock

`lock` opens a fullscreen SDL3 window in relative mouse mode, which locks the pointer through Wayland pointer constraints the way games do.

- **While locked:** 200 relative moves gave no positions, and the window received all of them as relative motion.
- **On unlock, with no input:** KWin moved the pointer to the client's cursor-position hint, and the script reported that position once.
- **After unlock:** every move was reported again.

The locked pointer stands still in KWin (`PointerInputRedirection::updatePosition` returns early while `m_locked`), so there is no position to record. Recording relative motion covers it, as map D4 already says.

### Lifecycle

- **Loading needs no consent and no setting.** `loadScript`, `run` and `unloadScript` worked from an unprivileged session client with no prompt. `~/.config/kwinrc` had the same checksum before and after.
- **`run()` returns after the script has run.** Called over D-Bus, `Script::run` defers its reply until the script has been read and evaluated (`src/scripting/scripting.cpp:166`). The first Layout call arrived about 3 ms after `loadScript`.
- **A crashed receiver leaves the script loaded.** After the receiver was killed with SIGKILL, `isScriptLoaded("x01-cursor")` was true and KWin kept paying for the calls (see CPU). The next start found the script by its plugin name and unloaded it before loading its own.
- **KWin restarts drop the script.** On a nested, virtual KWin in a private D-Bus session, the script was gone after KWin was killed and started again, and `org.kde.KWin` had a new owner. A daemon recording across a KWin restart has to watch that name and load the script again. The live session's KWin was never restarted.

## Other routes

- **KWin's own D-Bus interfaces.** Every object under `org.kde.KWin` was introspected for cursor, pointer, position and mouse members. The only position source is `org.freedesktop.a11y.PointerLocator.QueryPointer` on `/org/freedesktop/a11y/Manager`, new in Plasma 6.8.
  - It has no permission check, and it returns the pointer's sub-pixel position relative to the window under the pointer, with that window's pid.
  - It then sends a single `PointerPositionChanged` signal on the next move, as a throttle.
  - It needs no script, but the position is window-relative. Turning it into a layout position needs the window's geometry, which only a script can supply. It is an a11y interface whose keyboard half is limited to the Orca screen reader, so the same may happen to it.
  - The spike uses it only to read sub-pixel positions over its own fullscreen windows.
- **InputCapture portal.** Version 2 is present (`busctl --user introspect org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.InputCapture`). Pointer barriers report a position only when the pointer crosses a barrier, and then the portal takes the input away from the compositor until it is released. Neither suits passive recording. No session was opened, since that would have needed a consent prompt for a route that cannot serve.
- **Plasma 6.8 APIs.** No other interface exposes the global cursor position. A scripted KWin *effect* has a sub-pixel `effects.cursorPos`, but effects must be installed into KWin's data directory and enabled in its config, which is a setting. Wayland clients see the pointer only over their own surfaces, which is the route U03's overlay takes.

## Limitations

- **Precision is whole logical px.** `workspace.cursorPos` is `Cursor::pos().toPoint()` (`src/scripting/workspace_wrapper.cpp:148`). The error is up to 0.5 logical px: 0.625 physical px at scale 1.25, 1 physical px at scale 2.
- **The script cannot tell which device moved the pointer.** It also reports moves made by the program's own virtual mouse. Recording is refused while clicking or playing (R01), so the two never overlap.
- **Anyone in the session can load KWin scripts.** Any session process can also call the receiver's D-Bus name, so the receiver must accept calls only from the current owner of `org.kde.KWin`.
- **Pointer lock gives no positions.** While a game locks the pointer, recordings fall back to relative motion.
- **The layout must match at playback.** A position is only meaningful against the layout it was recorded on, so recordings must carry the layout size.
- **Only synthetic input was measured.** Every number above comes from C01's virtual mouse; a physical 1000 Hz mouse was not tested.
- **Shake-to-find is irrelevant.** KDE's "shake cursor to find it" effect enlarges the drawn cursor during fast moves. It does not change the position.

## Upstream reports (for the owner)

1. **KWin: `QueryPointer` adds a connection per call.**
   - Each call adds a new single-shot connection to `Cursor::posChanged` (`src/a11ymanager.cpp:203-208` in 6.8.2, unchanged on master at a2a7855509, 2026-11-09).
   - Observed: 100 calls without motion, then one move, gave 100 `PointerPositionChanged` signals. Expected one, since the interface documents the signal as a throttle.
   - Reproducer: `spikes/x01-cursor-position/harness/query_pointer_signals.py`, which calls `QueryPointer` N times, moves the pointer, and counts the signals.
   - Effect: a screen reader that polls while the pointer is still is flooded on the next move. It does not affect this program.
   - Not found in KDE Bugzilla for "PointerLocator", "QueryPointer" or "PointerPositionChanged" (searched 2026-11-10). The interface arrived in commit 23b0427864.
2. **KWin: `QueryPointer` has no permission check, while its sibling `KeyboardMonitor` is limited to Orca.** Any session client can learn the pointer position and the pid of the window under it. This is worth raising as a question rather than a bug.
3. **Wishlist: a sub-pixel `cursorPos` for KWin scripts.** `workspace.cursorPos` drops the sub-pixel part. A `QPointF` property would remove this program's only rounding error. Low priority: the current error stays under 1 physical px at scales up to 2.

## Commands

On the laptop, from `spikes/x01-cursor-position` (after `(cd inject && cargo build --release --locked)`), with `/tmp/auto-clicker-spike.lock` held:

```sh
for hz in 125 500 1000 2000; do python3 harness/x01.py latency --hz $hz --sweeps 3 --count 900; done
for rep in 1 2; do for hz in 0 125 1000 2000; do for mode in off on dead; do
  python3 harness/x01.py cpu --hz $hz --seconds 10 --mode $mode; done; done; done
./layouts.sh               # the playback table; restores eDP-1 0,0 @1.25 and DP-1 2048,0 @1
python3 harness/x01.py lock
./lifecycle.sh
```

Versions: `kwin_wayland --version`, `uname -r`, `pacman -Q libinput plasma-workspace qt6-base xdg-desktop-portal xdg-desktop-portal-kde libei`.
