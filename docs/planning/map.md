# Auto clicker: project map

## Purpose

An auto clicker with macro record and playback for a CachyOS machine running KDE Plasma, used by a child for games. It should match what common auto clickers offer, live in the system tray, and cost next to nothing while idle.

## Users and use cases

- **Repeating a routine:** record a sequence of mouse movements, clicks and key presses with their timing, then replay it once, N times or until stopped.
- **Minecraft-style games:** fast left or right clicking, holding a button or key down (attack, walk forward), toggled by a hotkey while the game has focus. These games often lock the pointer, so recorded movement must replay as relative motion there.

## Platform facts (checked 2026-11-09)

- Plasma 6.8 (October 2026) removed the X11 session; the dev box runs Plasma 6.8.2, KWin 6.8.2, xdg-desktop-portal-kde 6.8.2 and libei 1.6.0. **Wayland is the primary target from day one.** X11 games keep running under XWayland and must work.
- Wayland lets no client inject input into another or read the global pointer position. Two ways in:
  - **Kernel backend:** read real devices with evdev and inject through a uinput virtual device. Works on any compositor with no prompt. Injecting needs write access to `/dev/uinput` (a uaccess udev rule is enough); reading devices for hotkeys and recording needs read access to `/dev/input/event*`, in practice the `input` group, which means the program can technically read every keystroke.
  - **Portal backend:** libei through the RemoteDesktop portal for output, GlobalShortcuts portal for hotkeys, InputCapture portal for recording. KDE shows a permission prompt. InputCapture cannot record gameplay: it starts when the pointer crosses a barrier at a screen edge, not on a hotkey, and while it captures, input goes to the capturing app instead of the game. It is also unreliable on KDE as of 2026. Recording therefore stays on the kernel backend unless X02 finds another way.
- No existing Linux tool combines record and playback with a Plasma tray icon. theclicker-gui (Rust, uinput, SNI tray) and clickr (Rust, uinput) are references for the clicker part.

## Decisions

| # | Decision | Why |
| --- | --- | --- |
| D1 | Rust, one small daemon; tray via `ksni` (StatusNotifierItem) | Low idle cost; SNI is Plasma's native tray protocol |
| D2 | Both input backends in v1, behind one backend trait: kernel first (M1–M2), portal in M4, chosen automatically with a manual override. On the portal backend, recording is unavailable unless X02 finds a mechanism that leaves input with the game | Kernel works today with no prompts; the portal avoids the `input` group; InputCapture takes input away from the focused app by design |
| D3 | Recording captures clicks, keys, timing and mouse movement, including absolute cursor positions; a spike (X01) proves how to read the cursor on KWin first. If it fails, recordings keep relative motion and clicks play at the cursor | Under Wayland only the compositor knows the cursor position |
| D4 | Movement is recorded both as relative motion and, when available, absolute positions; playback uses relative motion while a game has locked the pointer | Minecraft-style games lock the pointer |
| D5 | Settings window in Slint, opened from the tray and loaded only while open; everyday use is tray plus hotkeys | Kid-friendly settings without raising idle cost |
| D6 | Hotkeys default to Ctrl+F6 (clicking on/off), Ctrl+F7 (record on/off), Ctrl+F8 (play/stop), rebindable | Ctrl combinations avoid clashing with game bindings |
| D7 | Recordings and config are TOML/JSON files under XDG paths (`~/.config/auto-clicker/`, `~/.local/share/auto-clicker/`) written atomically | Simple, inspectable, survives crashes |
| D8 | Install through a PKGBUILD with a udev rule and a systemd user service | CachyOS is Arch-based |
| D10 | Code reaches the machine only from this repository: v1 installs by the owner building a reviewed commit and has no network access; a later updater may fetch only signed releases of `tobylo/auto-clicker`, verified against the owner's pinned key. Dependencies from crates.io only, locked, and checked for advisories in CI | The program runs on a child's machine with access to input devices; nobody but the owner may be able to push code to it |
| D9 | Review services: Codex and CodeRabbit GitHub bots, CodeScene Code Health. No Entire | Owner's choice |
| D11 | The daemon runs on tokio's single-thread (`current_thread`) runtime. Timers sleep until absolute deadlines (`sleep_until`), and every wait selects on its deadline, the stop signal and shutdown. Tests use tokio's paused clock | One thread is enough for a tray program and costs nothing while idle; absolute deadlines keep delays from adding up; the paused clock checks schedules and "stops at once" exactly, without real waiting (#10) |

## Clicker options and measurements

- Clicker options: interval in ms or clicks per second, random jitter (up to ±50%), left/right/middle/back/forward, single or double click, hold a button or a key down, key-repeat mode, at cursor or at a fixed point, repeat count or until stopped.
- **Fastest stable interval: 10 ms** (C02, 2026-11-09, niri 26.04 on the dev box, uinput, release build; `scripts/measure-clicker.py`). A test window received 1000 of 1000 clicks at every interval tried, down to 2 ms. The limit is timing: tokio's timer has 1 ms resolution, so each period is off by up to about 0.9 ms, which stays under 10% of the interval from 10 ms up (10 ms: 9.3%, 12 ms: 7.7%, 15 ms: 6.0%) and exceeds it below (8 ms: 11.2%, 5 ms: 18.5%; measured on a local build with `INTERVAL_FLOOR` set to 1 ms, since the config rejects intervals under the floor). The mean period is exact. On the busy dev box, some runs at any interval also had one late period of 2 to 10 ms from scheduling; their 99th percentile stayed under 1 ms. The interval floor is therefore 10 ms (20 ms for a double click, which fits two clicks in each interval).
- **Idle cost of the engine** (same setup, 60 s each): clicking once a second used 0 CPU ticks (under 10 ms of CPU) and 209 context switches, about two wakeups per click (press, then release after the hold) plus a few more; holding a button used 0 ticks and 0 context switches.

## Milestones

| Milestone | Outcome |
| --- | --- |
| M0 Foundation | Cargo workspace, gates and CI; spikes X01 (KWin cursor position) and X02 (portal backend on Plasma 6.8) |
| M1 Auto clicker | Tray auto clicker with hotkeys on the kernel backend, installable |
| M2 Record and playback | Record and replay routines, with absolute positions if X01 succeeds |
| M3 Settings window | Clicker options, hotkey rebinding and a recordings list |
| M4 Portal backend | libei output, GlobalShortcuts hotkeys, backend auto-selection; recording on this backend only if X02 finds a way |

The issues, their codes and blockers are on [the Project](https://github.com/users/tobylo/projects/4); [github-map.json](github-map.json) maps codes to issue numbers.

## Deferred ideas

- A self-updater for signed releases of this repository (D10); not in v1.

## Open questions

- Does the son's machine also run Plasma 6.8? Assumed yes (CachyOS is rolling).
- Do the target games accept uinput clicks? Minecraft Java does as a native Wayland window: on the target laptop (Plasma 6.8.2), 10 of 10 slow right clicks and 1000 of 1000 at the 10 ms floor registered (Statistics, snowballs "Times Used"; C02, 2026-11-09). The XWayland run is #32: the launcher does not start without Wayland, so the game could not be forced onto XWayland.
