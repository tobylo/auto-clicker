# X01 spike: reading the cursor position on KWin

Evidence for [docs/decisions/0001-cursor-position.md](../../docs/decisions/0001-cursor-position.md). Not built by the workspace; nothing here is product code.

- `kwin/x01-cursor.js`: the KWin script. It reports `workspace.cursorPos` on every `cursorPosChanged`, and the layout on load and on change, by calling `io.github.tobylo.AutoClickerX01`.
- `inject/`: moves C01's virtual mouse (`auto-clicker-backend-kernel`) on command and timestamps each move on `CLOCK_MONOTONIC`. It has its own lock file. `nix` is already in the main lock tree through evdev; its `time` feature gives a safe `clock_gettime`.
- `harness/`: the receiver and the measurements (Python, dbus-python and PyGObject).
  - `x01.py layout | latency | cpu | mapping | lock | receive` selects the measurement.
  - `probe.py` opens a fullscreen GTK 4 window on one monitor, giving `QueryPointer` a known window to report against.
  - `query_pointer_signals.py` reproduces the upstream `QueryPointer` signal issue.
  - `lockwin.py` is an SDL3 window in relative mouse mode, which locks the pointer.
- `layouts.sh` runs the mapping check over six display layouts and restores the original layout on exit.
- `lifecycle.sh` runs the receiver-crash check and a KWin restart on a nested, virtual KWin.

## Running

On a Plasma session, from this directory:

```sh
(cd inject && cargo build --release --locked)
flock /tmp/auto-clicker-spike.lock python3 harness/x01.py latency --hz 1000 --sweeps 3 --count 900
flock /tmp/auto-clicker-spike.lock python3 harness/x01.py cpu --hz 1000 --seconds 10 --mode on   # on | off | dead
flock /tmp/auto-clicker-spike.lock python3 harness/x01.py mapping --monitors 2
flock /tmp/auto-clicker-spike.lock ./layouts.sh      # changes the display layout, then restores it
flock /tmp/auto-clicker-spike.lock python3 harness/x01.py lock
flock /tmp/auto-clicker-spike.lock ./lifecycle.sh
```

`layouts.sh` restores this laptop's own layout: eDP-1 at 0,0 and scale 1.25, DP-1 at 2048,0 and scale 1. Edit `restore` before running it anywhere else.
