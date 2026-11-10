"""What the script reports while a window has locked the pointer.

Moves the pointer before the lock, during it and after it, and prints how
many positions the script reported in each phase, the positions themselves,
and how much motion the locking window received.
"""

import os
import subprocess
import threading

from children import stop, wait_ready
from bus import HERE, Receiver, loaded_script, pump_for, session_bus
from inject import Injector, to_absolute, wait_for_pointer
from timing import current_layout

LOCKWIN = os.path.join(HERE, "harness", "lockwin.py")


def run():
    bus = session_bus()
    receiver = Receiver(bus)
    with loaded_script(bus), Injector() as injector:
        layout = current_layout(receiver)
        wait_for_pointer(injector, receiver, layout)
        centre = f"abs {to_absolute(1024, layout.width)} {to_absolute(640, layout.height)}"
        injector.run(centre)
        phase(receiver, injector, "before lock")
        injector.run(centre)
        window = start_window()
        try:
            injector.run("rel 1 1")
            phase(receiver, injector, "locked")
            print("window:", window.lines[-1] if window.lines else "no output")
            mark = len(receiver.samples)
        finally:
            stop(window.proc)
        pump_for(1.0)
        print(f"after unlock, with no input: {positions(receiver, mark)}")
        phase(receiver, injector, "after unlock")


def phase(receiver, injector, title):
    mark = len(receiver.samples)
    injector.run("stream-rel 500 200 2")
    pump_for(0.5)
    print(f"{title}: 200 relative moves -> {len(receiver.samples) - mark} positions, {positions(receiver, mark)[:3]}...{positions(receiver, mark)[-2:]}")


def positions(receiver, mark):
    return [(s.x, s.y) for s in receiver.samples[mark:]]


class Window:
    def __init__(self):
        self.proc = subprocess.Popen(["python3", LOCKWIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.lines = []
        self.ready = wait_ready(self.proc)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.lines.append(line.strip())


def start_window():
    window = Window()
    try:
        print("lock window:", window.ready)
        pump_for(1.0)
    except BaseException:
        stop(window.proc)
        raise
    return window
