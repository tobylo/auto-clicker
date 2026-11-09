#!/usr/bin/env python3
"""Measures the clicker engine on the kernel backend, by hand, in a Wayland session.

  measure-clicker.py rate MS [MS ...]   click-rate runs, one per interval
  measure-clicker.py idle               idle cost between clicks and while holding

Each run opens the device test's probe window fullscreen, runs the `clicker`
example (build it first with `cargo build --release --locked -p
auto-clicker-backend-kernel --example clicker`) with a generated config, and
counts the clicks the window receives. Periods come from the kernel's
microsecond timestamps on the virtual mouse's event node, since the window's
event times are whole milliseconds. Needs read access to /dev/input (the
`input` group) and a single enabled output. Leave the mouse and keyboard
alone while it runs.
"""

import errno
import os
import queue
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROBE = ROOT / "crates/backend-kernel/tests/fixtures/probe_window.py"
CLICKER = ROOT / "target/release/examples/clicker"

CLICKS = 1000
EV_KEY = 1
BTN_LEFT = 0x110
EVENT = struct.Struct("llHHi")  # struct input_event on 64-bit Linux
LINE_TIMEOUT = 10.0


def lines_of(stream, sink):
    for line in stream:
        sink.put(line.rstrip("\n"))


def spawn(args):
    """Starts a process and a queue of its stdout lines."""
    proc = subprocess.Popen(args, stdout=subprocess.PIPE, text=True, bufsize=1)
    lines = queue.Queue()
    threading.Thread(target=lines_of, args=(proc.stdout, lines), daemon=True).start()
    return proc, lines


def expect(lines, accept, what):
    """The first line `accept` takes, skipping others, within the timeout."""
    deadline = time.monotonic() + LINE_TIMEOUT
    while (left := deadline - time.monotonic()) > 0:
        try:
            line = lines.get(timeout=left)
        except queue.Empty:
            break
        found = accept(line)
        if found is not None:
            return found
    sys.exit(f"never saw {what}")


def drain(lines):
    """Every line that arrived so far."""
    out = []
    while True:
        try:
            out.append(lines.get_nowait())
        except queue.Empty:
            return out


def open_when_permitted(node):
    """Opens a new event node once udev has given it the `input` group."""
    deadline = time.monotonic() + LINE_TIMEOUT
    while True:
        try:
            return os.open(node, os.O_RDONLY)
        except PermissionError:
            if time.monotonic() > deadline:
                raise
            time.sleep(0.05)


def left_presses(data):
    """The timestamps (µs) of the left presses among raw input events."""
    for sec, usec, kind, code, value in EVENT.iter_unpack(data):
        if (kind, code, value) == (EV_KEY, BTN_LEFT, 1):
            yield sec * 1_000_000 + usec


def read_presses(node, presses, failures):
    """Appends the kernel timestamp (µs) of each left press on `node`, until
    the device goes away (ENODEV, when the clicker exits). Any other error is
    appended to `failures`, so it is not mistaken for missing clicks. Blocking
    reads keep up with every event."""
    try:
        fd = open_when_permitted(node)
    except OSError as err:
        failures.append(err)
        return
    try:
        while data := os.read(fd, EVENT.size * 64):
            presses.extend(left_presses(data))
    except OSError as err:
        if err.errno != errno.ENODEV:
            failures.append(err)
    finally:
        os.close(fd)


def write_config(directory, clicker):
    path = Path(directory) / "config.toml"
    path.write_text("version = 1\n[clicker]\n" + clicker)
    return path


class Session:
    """The probe window and one clicker run in it."""

    def __init__(self, config):
        self.procs = []
        try:
            self.start(config)
        except BaseException:
            # A failed start (the clicker rejected the config, say) must not
            # leave the fullscreen window behind.
            self.close()
            raise

    def start(self, config):
        self.window, self.window_lines = spawn(["python3", str(PROBE)])
        self.procs.append(self.window)
        expect(self.window_lines, lambda l: l == "ready" or None, "the window ready")
        self.clicker, self.clicker_lines = spawn([str(CLICKER), str(config), "1500"])
        self.procs.append(self.clicker)
        self.node = expect(self.clicker_lines, mouse_node, "the mouse node")

    def close(self):
        for proc in reversed(self.procs):
            stop(proc)


def mouse_node(line):
    return line.removeprefix("mouse ") if line.startswith("mouse ") else None


def stop(proc):
    """Ends a process: SIGTERM, then SIGKILL if it does not exit in time."""
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


def rate(interval_ms):
    with tempfile.TemporaryDirectory() as directory:
        config = write_config(
            directory,
            f'interval_ms = {interval_ms}\nrepeat = {CLICKS}\nposition = {{ x = 32768, y = 32768 }}\n',
        )
        session = Session(config)
        presses, failures = [], []
        reader = threading.Thread(target=read_presses, args=(session.node, presses, failures), daemon=True)
        reader.start()
        try:
            timeout = 5 + CLICKS * interval_ms / 1000 * 2
            session.clicker.wait(timeout=timeout)
            time.sleep(1.0)  # let the last events reach the window
        finally:
            reader.join(timeout=5)
            seen = sum(1 for l in drain(session.window_lines) if l == "press 1")
            session.close()
    if failures:
        sys.exit(f"reading the kernel's timestamps failed: {failures[0]}")
    periods = [b - a for a, b in zip(presses, presses[1:])]
    target = interval_ms * 1000
    mean = sum(periods) / len(periods) if periods else float("nan")
    errors = sorted(abs(p - target) for p in periods)
    worst = errors[-1] if errors else float("nan")
    p99 = errors[len(errors) * 99 // 100] if errors else float("nan")
    stable = seen == CLICKS and len(presses) == CLICKS and worst < target / 10
    print(
        f"interval {interval_ms} ms: window {seen}/{CLICKS}, kernel {len(presses)}/{CLICKS}, "
        f"mean period {mean / 1000:.3f} ms, worst error {worst / 1000:.3f} ms "
        f"({worst / target * 100:.1f}%), 99th percentile {p99 / 1000:.3f} ms, "
        f"{'stable' if stable else 'NOT stable'}"
    )


def cpu_and_switches(pid):
    """CPU ticks (user + system) and context switches over all threads."""
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])
    switches = 0
    for task in Path(f"/proc/{pid}/task").iterdir():
        for line in (task / "status").read_text().splitlines():
            if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                switches += int(line.split()[1])
    return ticks, switches


def idle(name, clicker, seconds=60):
    with tempfile.TemporaryDirectory() as directory:
        session = Session(write_config(directory, clicker))
        try:
            expect(session.clicker_lines, lambda l: l == "start" or None, "the run start")
            time.sleep(2)
            ticks0, switches0 = cpu_and_switches(session.clicker.pid)
            time.sleep(seconds)
            ticks1, switches1 = cpu_and_switches(session.clicker.pid)
        finally:
            session.close()
    hz = os.sysconf("SC_CLK_TCK")
    print(
        f"{name}: {ticks1 - ticks0} CPU ticks ({(ticks1 - ticks0) / hz * 1000:.0f} ms) "
        f"and {switches1 - switches0} context switches in {seconds} s"
    )


def main(argv):
    if len(argv) >= 2 and argv[0] == "rate":
        for ms in argv[1:]:
            rate(int(ms))
    elif argv == ["idle"]:
        idle("clicking once a second", 'interval_ms = 1000\nposition = { x = 32768, y = 32768 }\n')
        idle("holding the left button", 'mode = "hold_button"\nposition = { x = 32768, y = 32768 }\n')
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
