# Auto clicker: project map

## Purpose

An auto clicker with macro record and playback for KDE Plasma on CachyOS, made for playing games. It should match what common auto clickers offer, live in the system tray, and cost next to nothing while idle.

## Users and use cases

- **Repeating a routine:** record a sequence of mouse movements, clicks and key presses with their timing, then replay it once, N times or until stopped.
- **Minecraft-style games:** fast left or right clicking, holding a button or key down (attack, walk forward), toggled by a hotkey while the game has focus. These games often lock the pointer, so recorded movement must replay as relative motion there.

## Platform facts (checked 2026-11-09)

- Plasma 6.8 (October 2026) removed the X11 session; the target laptop runs Plasma 6.8.2, KWin 6.8.2, xdg-desktop-portal 1.22.1, xdg-desktop-portal-kde 6.8.2 and libei 1.6.0. **Wayland is the primary target from day one.** X11 games keep running under XWayland and must work.
- Wayland lets no client inject input into another or read the global pointer position. Two ways in:
  - **Kernel backend:** read real devices with evdev and inject through a uinput virtual device. Works on any compositor with no prompt. Injecting needs write access to `/dev/uinput` (a uaccess udev rule is enough); reading devices for hotkeys and recording needs read access to `/dev/input/event*`, in practice the `input` group, which means the program can technically read every keystroke.
  - **Portal backend** (X02, [decision 0002](../decisions/0002-portal-backend.md), Plasma 6.8.2): libei through the RemoteDesktop portal for output and the GlobalShortcuts portal for hotkeys both work. The RemoteDesktop prompt shows once and its restore token survives restarts and reboots. The GlobalShortcuts dialog shows once, but the program must bind on every start and cannot change a binding itself (System Settings does, through `ConfigureShortcuts`). An activation names the shortcut and no device, so with portal hotkeys the program never presses a hotkey's key (P02, #20). KWin releases everything a libei device held when the program dies or the session closes. GlobalShortcuts refuses a program without an app id, and RemoteDesktop scopes its grant by it: the package installs `io.github.tobylo.AutoClicker.desktop` and the user service is `auto-clicker.service` (C05), so the daemon calls `Registry.Register` before its first portal session (P01, #19). InputCapture cannot record gameplay: capture starts when the pointer crosses a barrier at a screen edge, and while it lasts the focused window receives no input at all (measured: 0 events). No portal or KWin interface observes input while leaving it with the game, so recording stays on the kernel backend.
  - Detection works per capability (output, hotkeys, recording). The target laptop is the mixed case: `/dev/uinput` is writable through uaccess, but `/dev/input/event*` is not readable without the `input` group, so output has a kernel route, hotkeys only a portal route, and recording none.
- No existing Linux tool combines record and playback with a Plasma tray icon. theclicker-gui (Rust, uinput, SNI tray) and clickr (Rust, uinput) are references for the clicker part.

## Decisions

| # | Decision | Why |
| --- | --- | --- |
| D1 | Rust, one small daemon; tray via `ksni` (StatusNotifierItem) | Low idle cost; SNI is Plasma's native tray protocol |
| D2 | Both input backends in v1, behind one backend trait: kernel first (M1–M2), portal in M4, chosen automatically with a manual override, per capability (`[input] output = "kernel" \| "portal"`, P01; `hotkeys = "kernel" \| "portal"`, P02; P04 adds the automatic choice). Portal hotkeys use one GlobalShortcuts session for the daemon's life, bound again when the portal service returns; System Settings owns the triggers from the first bind on (P02). The portal session opens for each run and closes after it, so the desktop shows remote control only while clicking (P01). On the portal backend, recording is unavailable: X02 found no mechanism that leaves input with the game ([0002](../decisions/0002-portal-backend.md)) | Kernel works today with no prompts; the portal avoids the `input` group; InputCapture takes input away from the focused app by design |
| D3 | Recording captures clicks, keys, timing and mouse movement, including absolute cursor positions. X01 settled how ([decision 0001](../decisions/0001-cursor-position.md)): while recording, the daemon loads a KWin script that reports `workspace.cursorPos` (whole logical px) over D-Bus, and unloads it at stop. Playback moves the absolute axes to `round(x * 65536 / W)`, with W and H the size of KWin's layout, landing within 0.65 physical px per axis of where the real pointer was on one and two monitors at scales 1 to 1.5 (the bound grows with the scale; see the decision's limitations) | Under Wayland only the compositor knows the cursor position; KWin scripts read it with no consent, about 1 ms behind at most, and cost nothing while idle |
| D4 | Movement is recorded both as relative motion and, when available, absolute positions; playback uses relative motion while a game has locked the pointer | Minecraft-style games lock the pointer |
| D5 | Settings window in Slint, opened from the tray and loaded only while open; everyday use is tray plus hotkeys | Settings without editing a file, and without raising idle cost |
| D6 | Hotkeys default to Ctrl+Shift+F6 (clicking on/off), Ctrl+Shift+F7 (record on/off), Ctrl+Shift+F8 (play/stop), rebindable: in the config on the kernel route, in System Settings on the portal route | Free in stock Plasma, where Ctrl+F7, Ctrl+F9, Ctrl+F10 and Super+F1 to F7 are taken (X02, P02); Minecraft binds nothing to F6, F7 or F8 |
| D7 | Recordings and config are TOML/JSON files under XDG paths (`~/.config/auto-clicker/`, `~/.local/share/auto-clicker/`) written atomically. Recordings (R01) are JSON, version 1, one file each in `recordings/`, held in memory while recording and written once at the stop, renamed into place without replacing (`RENAME_NOREPLACE`). One reader of `/dev/input` serves hotkeys and recording; it opens mice only while recording | Simple, inspectable, survives crashes; idle stays free |
| D8 | Install through a PKGBUILD with a udev rule and a systemd user service | CachyOS is Arch-based |
| D10 | Code reaches the machine only from this repository: v1 installs by the owner building a reviewed commit and has no network access; a later updater may fetch only signed releases of `tobylo/auto-clicker`, verified against the owner's pinned key. Dependencies from crates.io only, locked, and checked for advisories in CI | The program has access to input devices; nobody but the owner may be able to push code to a machine that runs it |
| D9 | Review services: Codex and CodeRabbit GitHub bots, CodeScene Code Health. No Entire | Owner's choice |
| D11 | The daemon runs on tokio's single-thread (`current_thread`) runtime. Timers sleep until absolute deadlines (`sleep_until`), and every wait selects on its deadline, the stop signal and shutdown. Tests use tokio's paused clock | One thread is enough for a tray program and costs nothing while idle; absolute deadlines keep delays from adding up; the paused clock checks schedules and "stops at once" exactly, without real waiting (#10) |

## Clicker options and measurements

- Clicker options: interval in ms or clicks per second, random jitter (up to ±50%), left/right/middle/back/forward, single or double click, hold a button or a key down, key-repeat mode, at cursor or at a fixed point, repeat count or until stopped.
- **Fastest stable interval: 10 ms** (C02, 2026-11-09, niri 26.04 on the dev box, uinput, release build; `scripts/measure-clicker.py`). A test window received 1000 of 1000 clicks at every interval tried, down to 2 ms. The limit is timing: tokio's timer has 1 ms resolution, so each period is off by up to about 0.9 ms, which stays under 10% of the interval from 10 ms up (10 ms: 9.3%, 12 ms: 7.7%, 15 ms: 6.0%) and exceeds it below (8 ms: 11.2%, 5 ms: 18.5%; measured on a local build with `INTERVAL_FLOOR` set to 1 ms, since the config rejects intervals under the floor). The mean period is exact. On the busy dev box, some runs at any interval also had one late period of 2 to 10 ms from scheduling; their 99th percentile stayed under 1 ms. The interval floor is therefore 10 ms (20 ms for a double click, which fits two clicks in each interval).
- **Idle cost of the engine** (same setup, 60 s each): clicking once a second used 0 CPU ticks (under 10 ms of CPU) and 209 context switches, about two wakeups per click (press, then release after the hold) plus a few more; holding a button used 0 ticks and 0 context switches.
- **Playback** (R02, #15). A recording plays through the same run machinery as clicking (one wait that selects on its deadline, stop and shutdown; everything released at the end). Each event is due at `start + (round * duration + offset) / speed`, computed from the run's start every time, so nothing adds up; events already due fire in order, and a late one does not move the rest. Speed is 0.25 to 4, repeats are a count or until stopped, and a recording under 100 ms plays once only. Measured 2026-11-10 on niri 26.04 on the dev box, uinput, debug build (`crates/backend-kernel/tests/playback.rs`): replaying a 20-event routine twice at speed 1.5 and recording the replay from the virtual devices, every event was within 1.3 ms of its place, over two runs. Replayed relative motion passes through the compositor's pointer acceleration: 400 units moved the pointer 397 px recorded slowly and played at quarter speed, and 790 px recorded or played fast (niri's default adaptive profile; Plasma 6.8.2 on the laptop gave the same figures, 397 to 795 px). Pointer-locked games get the raw deltas. A key the user holds while playback presses and releases the same key stays down for the window until the user lets go on Plasma 6.8.2; on niri the window sees it come up at playback's release.
- **Idle cost of the daemon** (C04, 2026-11-10, niri 26.04 on the dev box, release build, `scripts/measure-daemon.py`, 60 s with the tray icon shown, hotkeys read and logind followed): 0 CPU ticks, 0 context switches, 1 thread, 5.8 MiB resident. The release binary is 3.7 MB.

## Milestones

| Milestone | Outcome |
| --- | --- |
| M0 Foundation | Cargo workspace, gates and CI; spikes X01 (KWin cursor position) and X02 (portal backend on Plasma 6.8) |
| M1 Auto clicker | Tray auto clicker with hotkeys on the kernel backend, installable |
| M2 Record and playback | Record and replay routines, with absolute positions if X01 succeeds |
| M3 Settings window | Clicker options, hotkey rebinding and a recordings list |
| M4 Portal backend | libei output, GlobalShortcuts hotkeys, backend auto-selection; recording stays on the kernel backend (X02: InputCapture takes the input from the game) |

The issues, their codes and blockers are on [the Project](https://github.com/users/tobylo/projects/4); [github-map.json](github-map.json) maps codes to issue numbers.

## Deferred ideas

- A self-updater for signed releases of this repository (D10); not in v1.

## Open questions

- Does the next machine the program is installed on also run Plasma 6.8? Assumed yes (CachyOS is rolling).
- Do the target games accept uinput clicks? Minecraft Java does as a native Wayland window: on the target laptop (Plasma 6.8.2), 10 of 10 slow right clicks and 1000 of 1000 at the 10 ms floor registered (Statistics, snowballs "Times Used"; C02, 2026-11-09). The XWayland run is #32: the launcher does not start without Wayland, so the game could not be forced onto XWayland.
