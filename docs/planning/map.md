# Auto clicker: project map

## Purpose

An auto clicker with macro record and playback for a CachyOS machine running KDE Plasma, used by a child for games. It should match what common auto clickers offer, live in the system tray, and cost next to nothing while idle.

## Users and use cases

- **Repeating a routine:** record a sequence of mouse movements, clicks and key presses with their timing, then replay it once, N times or until stopped.
- **Minecraft-style games:** fast left or right clicking, holding a button or key down (attack, walk forward), toggled by a hotkey while the game has focus. These games often lock the pointer, so recorded movement must replay as relative motion there.

## Platform facts (checked 2026-11-09)

- Plasma 6.8 (October 2026) removed the X11 session; the dev box runs Plasma 6.8.2, KWin 6.8.2, xdg-desktop-portal-kde 6.8.2 and libei 1.6.0. **Wayland is the primary target from day one.** X11 games keep running under XWayland and must work.
- Wayland lets no client inject input into another or read the global pointer position. Two ways in:
  - **Kernel backend:** read real devices with evdev and inject through a uinput virtual device. Works on any compositor with no prompt; needs the user in the `input` group (or a uaccess ACL on `/dev/uinput`), and can technically read every keystroke.
  - **Portal backend:** libei through the RemoteDesktop portal for output, GlobalShortcuts portal for hotkeys, InputCapture portal for recording. KDE shows a permission prompt; InputCapture is still unreliable on KDE as of 2026.
- No existing Linux tool combines record and playback with a Plasma tray icon. theclicker-gui (Rust, uinput, SNI tray) and clickr (Rust, uinput) are references for the clicker part.

## Decisions

| # | Decision | Why |
| --- | --- | --- |
| D1 | Rust, one small daemon; tray via `ksni` (StatusNotifierItem) | Low idle cost; SNI is Plasma's native tray protocol |
| D2 | Both input backends in v1, behind one backend trait: kernel first (M1–M2), portal in M4, chosen automatically with a manual override | Kernel works today with no prompts; the portal avoids the `input` group |
| D3 | Recording captures clicks, keys, timing and mouse movement, including absolute cursor positions; a spike (X01) proves how to read the cursor on KWin first. If it fails, recordings keep relative motion and clicks play at the cursor | Under Wayland only the compositor knows the cursor position |
| D4 | Movement is recorded both as relative motion and, when available, absolute positions; playback uses relative motion while a game has locked the pointer | Minecraft-style games lock the pointer |
| D5 | Settings window in Slint, opened from the tray and loaded only while open; everyday use is tray plus hotkeys | Kid-friendly settings without raising idle cost |
| D6 | Hotkeys default to Ctrl+F6 (clicking on/off), Ctrl+F7 (record on/off), Ctrl+F8 (play/stop), rebindable | Ctrl combinations avoid clashing with game bindings |
| D7 | Recordings and config are TOML/JSON files under XDG paths (`~/.config/auto-clicker/`, `~/.local/share/auto-clicker/`) written atomically | Simple, inspectable, survives crashes |
| D8 | Install through a PKGBUILD with a udev rule and a systemd user service | CachyOS is Arch-based |
| D9 | Review services: Codex and CodeRabbit GitHub bots, CodeScene Code Health. No Entire | Owner's choice |

## Defaults (not yet measured)

- Clicker options: interval in ms or clicks per second, random jitter, left/right/middle, single or double click, hold-down mode, key-repeat mode, at cursor or at a fixed point, repeat count or until stopped.
- The fastest reliable interval is measured in C02, not assumed; one tool's README claims 25 ms, which is unverified.

## Milestones

| Milestone | Outcome |
| --- | --- |
| M0 Foundation | Cargo workspace, gates and CI; spikes X01 (KWin cursor position) and X02 (portal backend on Plasma 6.8) |
| M1 Auto clicker | Tray auto clicker with hotkeys on the kernel backend, installable |
| M2 Record and playback | Record and replay routines, with absolute positions if X01 succeeds |
| M3 Settings window | Clicker options, hotkey rebinding and a recordings list |
| M4 Portal backend | libei output, GlobalShortcuts hotkeys, InputCapture recording, backend auto-selection |

The issues, their codes and blockers are on [the Project](https://github.com/users/tobylo/projects/4); [github-map.json](github-map.json) maps codes to issue numbers.

## Open questions

- Does the son's machine also run Plasma 6.8? Assumed yes (CachyOS is rolling).
- Do the target games accept uinput clicks? Expected yes; C02 checks Minecraft Java under XWayland and native Wayland.
