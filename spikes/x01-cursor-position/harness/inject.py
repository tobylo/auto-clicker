"""Drives the Rust injector (C01's virtual mouse) without blocking the loop."""

import os
import subprocess
from dataclasses import dataclass

from gi.repository import GLib

from bus import HERE, pump_until

INJECT = os.path.join(HERE, "inject", "target", "release", "x01-inject")


class Injector:
    """One injector process; its replies are read from the main loop."""

    def __init__(self):
        self.proc = subprocess.Popen([INJECT], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        os.set_blocking(self.proc.stdout.fileno(), False)
        self._buffer = b""
        self._stamps = []
        self._done = True
        GLib.io_add_watch(self.proc.stdout.fileno(), GLib.IO_IN | GLib.IO_HUP, self._readable)

    def _readable(self, fd, _condition):
        chunk = os.read(fd, 65536)
        if not chunk:
            return False
        self._buffer += chunk
        *lines, self._buffer = self._buffer.split(b"\n")
        for line in lines:
            self._line(line.decode())
        return True

    def _line(self, line):
        words = line.split()
        if words[:1] == ["t"]:
            self._stamps.append(int(words[2]))
        elif words == ["done"]:
            self._done = True

    def run(self, command, timeout=120):
        """Sends one command and returns the move times (monotonic ns)."""
        self._stamps, self._done = [], False
        self.proc.stdin.write((command + "\n").encode())
        self.proc.stdin.flush()
        if not pump_until(lambda: self._done or self.proc.poll() is not None, timeout):
            raise RuntimeError(f"injector gave no reply to {command!r}")
        if not self._done:
            raise RuntimeError(f"injector exited during {command!r}")
        return self._stamps

    def close(self):
        """Ends the process, which destroys the virtual devices. One still
        running a command is killed at once, so the pointer stops moving."""
        if self.proc.poll() is None and not self._done:
            self.proc.kill()
        if self.proc.poll() is None:
            try:
                self.proc.stdin.write(b"quit\n")
                self.proc.stdin.close()
                self.proc.wait(timeout=5)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.proc.kill()
                self.proc.wait(timeout=5)

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        self.close()


@dataclass
class Session:
    """What every measurement works with: the mouse, the receiver, the layout."""

    injector: Injector
    receiver: object
    layout: object


def to_absolute(pos, size):
    """A layout position (logical px) to the absolute device's 0..=65535."""
    return max(0, min(65535, round(pos * 65536 / size)))


def wait_for_pointer(injector, receiver, layout, timeout=10):
    """The new device reaches KWin a moment after creation with no signal:
    move between two points until the script reports a move."""
    points = [(layout.width // 2, layout.height // 2), (layout.width // 4, layout.height // 2)]
    for i in range(int(timeout / 0.25)):
        x, y = points[i % 2]
        before = len(receiver.samples)
        injector.run(f"abs {to_absolute(x, layout.width)} {to_absolute(y, layout.height)}")
        if pump_until(lambda: len(receiver.samples) > before, 0.25):
            return
    raise RuntimeError("KWin never moved the pointer for the virtual mouse")
