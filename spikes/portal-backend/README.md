# X02 spike: the portal backend on Plasma 6.8

Throwaway code behind [docs/decisions/0002-portal-backend.md](../../docs/decisions/0002-portal-backend.md). It is not part of the workspace, nothing in the product depends on it, and it is not maintained.

## Dependencies

All from crates.io, locked in `Cargo.lock`, pure Rust, with no system libraries:

- `ashpd` 0.14.0 (zbus 6, tokio): the RemoteDesktop, GlobalShortcuts and InputCapture portals and the host app registry. Default features are off. `screencast` is on because 0.14.0's `remote_desktop` module does not build without it.
- `reis` 0.7.1 (tokio): the libei protocol, `#![forbid(unsafe_code)]`.
- `tokio`, `futures-util`, `enumflags2` (needed for reis's capability flags), `rustix` (`CLOCK_MONOTONIC` stamps).

## Running it

On the laptop, in the Plasma session's environment (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`), with `probe_window.py` copied from `crates/backend-kernel/tests/fixtures/`:

```sh
cargo build --release --locked
./target/release/portal-backend-spike detect
```

The portals need an app id (see the decision record). Install a `.desktop` file for the spike-only id `io.github.tobylo.AutoClickerX02` in `~/.local/share/applications/`, then start the spike either through `run-unit.sh` (a systemd user service named `app-<id>.service`, the daemon's own launch shape) or with `--app-id io.github.tobylo.AutoClickerX02` (`org.freedesktop.host.portal.Registry.Register`).

Wrap anything that injects input or opens a portal session in `flock /tmp/auto-clicker-spike.lock`: the laptop is shared with other spikes.

| Command | What it does |
| --- | --- |
| `detect` | Opens `/dev/uinput` and every `/dev/input/event*`, asks the portals for their versions, and names a route for output, hotkeys and recording. |
| `output TOKEN demo` | Starts a RemoteDesktop session (restoring from the token file), connects to EIS, and sends absolute motion, left, right and middle clicks, a key, relative motion and scrolling, each once. |
| `output TOKEN rate N MS` | N left clicks on absolute deadlines, MS apart. |
| `output TOKEN hold button\|key kill\|close\|eis\|stop\|exit` | Holds a button or key, then ends by SIGKILL (sent by the harness), by closing the portal session, by disconnecting the EIS socket, by stopping emulation, or by exiting without releasing. Afterwards it writes once more and reports the result. |
| `output TOKEN combo` | Ctrl+F6 through libei, to test whether GlobalShortcuts reacts to the program's own input. |
| `output TOKEN locked` | Relative motion and clicks for a game that has locked the pointer. |
| `output TOKEN grant` | Starts a session and closes it: the permission prompt alone. |
| `shortcuts list\|bind\|rebind\|configure SECS` | Lists the session's shortcuts, binds Ctrl+F6/F7/F8 (`rebind`: asks for Ctrl+Shift+F6/F7/F8 instead; `configure`: binds, then calls ConfigureShortcuts), then prints activations and changes for SECS. |
| `capture SECS` | InputCapture with a pointer barrier along the top edge. It prints event kinds and counts, never key codes, and hands input back 3 s after each activation. |

`hold.sh WHAT HOW` runs one hold case and sends the SIGKILL itself for `kill`. `capture_window.py PREFIX` counts the probe window's pointer events inside and outside each capture of a `capture` run.

`run-probe.sh PREFIX COMMAND…` opens the probe window with `WAYLAND_DEBUG=1` and runs the command while the window has focus. `wayland_events.py PREFIX.wayland` reads the raw `wl_pointer.button` and `wl_keyboard.key` events from that log: these show what the window received, including releases sent by the compositor.

Every run carries `--deadline` (60 s by default) and `run-unit.sh` sets `RuntimeMaxSec`, so a lost SSH session cannot leave the spike holding input.
