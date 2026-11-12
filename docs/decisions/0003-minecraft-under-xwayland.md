# 0003: Minecraft Java under XWayland

Status: accepted (#32, 2026-11-12). Answers the XWayland half of the scope gate's Minecraft criterion (#2).

## Decision

Minecraft Java takes the program's clicks and hotkey holds under XWayland as it does as a native Wayland window. The kernel route needs nothing specific to XWayland.

| Check | Native Wayland | XWayland |
| --- | --- | --- |
| 1000 right clicks at the 10 ms floor (C02) | 1000 of 1000 (2026-11-09) | 1000 of 1000 |
| Ctrl+Shift+F6 hold, fullscreen | mines continuously (#66, 2026-11-12) | mines continuously, 5 of 5 holds |
| The game still gets the hotkey's keys | yes (#61) | yes |

## Setup

- The target laptop, CachyOS, kernel 7.3.3-1-cachyos. Plasma 6.8.2 on Wayland: KWin 6.8.2, plasma-workspace 6.8.2, Xwayland 24.1.15. Two outputs: eDP-1 and DP-1.
- Minecraft Java 26.3 from the official launcher (`minecraft-launcher`, package wrapper `minecraft-launcher.sh`), creative world.
- Built from c8252c8 (release): `examples/clicker` for the clicks, the daemon for the hotkeys, both started from SSH. The daemon reported "Clicks through /dev/uinput, hotkeys from the keyboards".
- A logger read every `/dev/input/event*` node and logged only Ctrl, Shift, F6 and the mouse buttons, with each device's name.

## Forcing XWayland

Minecraft 26.3 no longer uses GLFW: it opens its window through SDL 3 (LWJGL 3.4.3, `lwjgl-sdl`), so SDL's video driver decides the window system. The launcher was started with the Plasma session's environment plus `SDL_VIDEO_DRIVER=x11` (and the older spelling `SDL_VIDEODRIVER=x11`), with `WAYLAND_DISPLAY` left set. The game inherits the variables and `latest.log` says:

```
[14:30:51] [Render thread/INFO]: SDL [VIDEO]: Detected XWayland
[14:30:52] [Render thread/INFO]: Created window using SDL video driver: x11
```

The launcher profile's JVM arguments cannot set an environment variable, so the launcher's environment is the place to set it. No profile was changed.

The launcher must be started through the package's `minecraft-launcher.sh`, which deletes `~/.minecraft/webcache2/CEF_VERSION` to work around a known launcher crash (MCL-25003, KDE bug 501866). Started directly as `minecraft-launcher`, it shows "Whoops! An unexpected issue occurred" and logs `terminate called after throwing an instance of 'std::bad_function_call'`, with or without `WAYLAND_DISPLAY` and the SDL variable. That is the crash #32's first try (2026-11-09) hit, so it did not show that the launcher needs Wayland.

## Clicks (C02)

`examples/clicker` with `mode = "click"`, `button = "right"`, `interval_ms = 10`, `repeat = 1000`, snowballs in hand, facing open sky. "Times Used" for snowballs went from 1030 to 2030. The logger saw 1000 `BTN_RIGHT` presses and 1000 releases on "auto-clicker virtual mouse" from 14:33:32.867 to 14:33:42.863, and no button from any other device. A first run, with the pause menu still open, sent the same 1000 clicks and the statistic did not move: the game counts only clicks it receives while it is being played.

## Hotkey holds (#61's check)

`mode = "hold_button"`, `button = "left"`, Ctrl+Shift+F6 to start and stop, with the game in fullscreen and facing a wall or the floor.

| Start | Condition | Button held | Result |
| --- | --- | --- | --- |
| 14:41:52.249 | wall | 5.18 s | kept mining |
| 14:43:02.722 | floor, after Meta+L and unlocking (locker up at 14:42:32) | 1.27 s | kept mining |
| 14:43:19.234 | floor, F6 still down 2.6 s after Ctrl and Shift came up | 3.64 s | kept mining |
| 14:51:25.007 | wall, game restarted, Ctrl+Shift held 1.4 s before F6 | 5.10 s | kept mining |
| 14:51:41.631 | wall, Ctrl+Shift held 5.8 s after F6 | 2.11 s | kept mining |

Each hold was one `BTN_LEFT down` on the virtual mouse 0.5 to 1.8 ms after the last of Ctrl and Shift came up, and one `BTN_LEFT up` at the stopping F6. While Ctrl+Shift were held, the player crouched, so the game got the keys the program reads as a hotkey. After every run, no virtual device was left (`/sys/class/input/*/name`) and no daemon, clicker or logger was running.
