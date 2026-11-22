# O01a spike: an on-screen display above a fullscreen game

Evidence for [docs/decisions/0005-on-screen-display.md](../../docs/decisions/0005-on-screen-display.md). Not built by the workspace; nothing here is product code.

## What is here

- `o01-probe` (Rust, `src/`): the prototype.
  - `globals` lists what the compositor offers.
  - `outputs` lists each screen's name, logical size and place.
  - `show` is the display: one layer-shell surface per screen and picture, on the overlay layer, with no keyboard interactivity and an empty input region. It is drawn into shared memory from an anonymous file, at the fractional scale through a viewport. It takes commands on stdin: `number N`, `badge`, `both N`, `pattern` (one-pixel stripes), `none`, `stats` and `quit`.
  - `window locked|free [OUTPUT]` is a fullscreen test window that holds a persistent pointer lock (or not) and prints what it hears.
  - `draw W H SCALE [SECONDS]` times one draw of the countdown for a screen of that size: "88" by default, `5940` for "99m", the widest.
  - Every line printed starts with seconds on `CLOCK_MONOTONIC`.
- `run.py WINDOW SCENARIO [OUTPUT]` drives one case: a test window, the display above it, and virtual devices (`uinput.py`) that inject only into that window. It injects nothing more once the window's latest lines say it lost focus or its lock, releases and destroys its devices on every way out, and calls a case VOID when the display never drew or ended with an error. WINDOW is `probe-`, `sdl-`, `x11-` or `xlib-` with `locked` or `free`. SCENARIO is `control`, `number`, `badge`, `both`, `appear`, `swap`, `remove` or `badge-remove`. `ONE_WAY=1` makes every move (+1, 0), so merged moves keep their sum. `FULLSCREEN_CMD` runs a command once the window has focus (niri's `fullscreen-window` for an X11 window under xwayland-satellite). `TRACE=FILE` appends every line the window and the display printed.
- `sdlwin.py` is an SDL 3 test window, with the Wayland or X11 driver.
- `xwin.py` is an X11 window that grabs the pointer, hides the cursor and warps it back to its centre, which Xwayland turns into a pointer lock.
- `cycle.py` shows the display above a game on a timer while someone plays. `frames.py` cuts a MangoHud frame log into its phases.
- `mclaunch.py VERSION` starts an installed Minecraft Java version without the launcher, offline, from `MC_DIR` (default `~/.minecraft`).
- `session/` holds the shell scripts the runs on the laptop used, fed to `ssh … bash -s` after a `CASES=…` or `CMD=…` line:
  - `plasma-cases.sh` and `hyprland-cases.sh` run `run.py` cases with the installed daemon stopped around them; `hyprland-env.sh` supplies the Hyprland session's environment.
  - `minecraft-start.sh` starts Minecraft with MangoHud logging every frame (`DRIVER`, `DIR`, `LOGSECS`).
  - `kwin-restart.sh` kills a nested KWin under `kwin_wayland_wrapper` while the display shows on it.
  - `lock.sh` keeps the display showing across a screen lock.

## Running

Build the probe, then run cases on the session's compositor:

```sh
cargo build --release --locked
ONE_WAY=1 python3 run.py probe-locked control
ONE_WAY=1 python3 run.py probe-locked number
```

Elsewhere than the session's own machine, copy the Python files to `/tmp/o01a` and the probe to `/tmp/o01a/target/release/o01-probe` there (where the scripts look for it), and feed it a script from `session/` with `ssh … bash -s`.

Each run injects input into its own test window only, and only once that window has keyboard focus (and, for the probe's window, the lock). Run it only on a screen nobody is using.

In MangoHud, `no_display` also turns logging off: give it a small visible counter (`fps_only`). It writes its log when `log_duration` ends or on Shift+F2, not when the game exits.
