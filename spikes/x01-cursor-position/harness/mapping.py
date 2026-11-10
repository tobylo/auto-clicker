"""The playback check: does an absolute move to a recorded position land on it?

For each anchor point: move there, nudge the pointer with relative motion so
it rests at a fractional position like a real mouse, read the original
position (QueryPointer, sub-pixel) and the recorded one (the script, whole
logical px), move away, replay the recorded one through the formula, and read
where it landed. Fullscreen probe windows make QueryPointer's window-relative
position global: each probe's origin is its output's origin.
"""

import os
import subprocess

from gi.repository import GLib

from children import stop, wait_ready
from bus import HERE, Receiver, loaded_script, pump_for, pump_until, query_pointer, session_bus
from inject import Injector, Session, to_absolute, wait_for_pointer
from stats import summary

PROBE = os.path.join(HERE, "harness", "probe.py")


def run(monitors, jiggle):
    bus = session_bus()
    receiver = Receiver(bus)
    probes = []
    try:
        for index in range(monitors):
            probes.append(start_probe(index))
        with loaded_script(bus), Injector() as injector:
            layout = receiver_layout(receiver)
            wait_for_pointer(injector, receiver, layout)
            check = Check(bus, Session(injector, receiver, layout), {p.pid: p for p in probes})
            rows = [check.anchor(point, jiggle) for point in anchors(layout)]
            uncovered = [(point, check.landing(point)) for point in dead_zone(layout)]
            offsets = check.surface_offsets
    finally:
        for probe in probes:
            stop_probe(probe)
    report(layout, rows)
    print(f"  probe surface-local vs QueryPointer: {summary(offsets, 'px')}")
    for point, landed in uncovered:
        print(f"  outside every output: target {point} landed {landed}")


class Probe:
    """A probe window; keeps the last surface-local motion it printed."""

    def __init__(self, index):
        self.proc = subprocess.Popen(["python3", PROBE, str(index)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        _, x, y = wait_ready(self.proc).split()
        self.origin = (int(x), int(y))
        self.pid = self.proc.pid
        self.motion = None
        self._buffer = b""
        os.set_blocking(self.proc.stdout.fileno(), False)
        GLib.io_add_watch(self.proc.stdout.fileno(), GLib.IO_IN | GLib.IO_HUP, self._readable)

    def _readable(self, fd, _condition):
        chunk = os.read(fd, 65536)
        if not chunk:
            return False
        *lines, self._buffer = (self._buffer + chunk).split(b"\n")
        for words in (line.split() for line in lines):
            if words[:1] == [b"motion"]:
                self.motion = (float(words[1]), float(words[2]))
        return True


def start_probe(index):
    return Probe(index)


def stop_probe(probe):
    stop(probe.proc)


def receiver_layout(receiver):
    if not pump_until(lambda: receiver.layouts, 5):
        raise RuntimeError("no layout from the script")
    return receiver.layouts[-1]


def anchors(layout):
    """A grid on every output, its four corner pixels, and the dead zone."""
    points = []
    for out in layout.outputs:
        points += grid(out)
        right, bottom = out["x"] + out["w"] - 1, out["y"] + out["h"] - 1
        points += [(out["x"], out["y"]), (right, out["y"]), (out["x"], bottom), (right, bottom)]
    return points


def grid(out):
    xs = [out["x"] + out["w"] * i // 6 for i in range(1, 6)]
    ys = [out["y"] + out["h"] * j // 4 for j in range(1, 4)]
    return [(x, y) for x in xs for y in ys]


def dead_zone(layout):
    """Points inside the bounding box that no output covers."""
    w, h = layout.width, layout.height
    candidates = [(x, y) for x in (w // 8, w // 2, w - w // 8) for y in (h // 8, h - h // 16)]
    return [p for p in candidates if output_at(layout, *p) is None]


def output_at(layout, x, y):
    for out in layout.outputs:
        if out["x"] <= x < out["x"] + out["w"] and out["y"] <= y < out["y"] + out["h"]:
            return out
    return None


class Check:
    def __init__(self, bus, session, probe_pids):
        self.bus, self.receiver, self.injector = bus, session.receiver, session.injector
        self.layout, self.probe_pids = session.layout, probe_pids
        self.surface_offsets = []

    def move(self, command):
        before = self.receiver.last()
        self.injector.run(command)
        pump_until(lambda: self.receiver.last() is not before, 1.0)
        pump_for(0.03)

    def move_to(self, x, y):
        self.move(f"abs {to_absolute(x, self.layout.width)} {to_absolute(y, self.layout.height)}")

    def landing(self, point):
        self.move_to(self.layout.width // 2 + 37, self.layout.height // 3)
        self.move_to(*point)
        last = self.receiver.last()
        return (last.x, last.y)

    def global_position(self):
        """QueryPointer made global; None when no probe is under the pointer."""
        pid, rel_x, rel_y = query_pointer(self.bus)
        probe = self.probe_pids.get(pid)
        if probe is None:
            return None
        out = output_at(self.layout, *probe.origin)
        self.surface_offsets.append(surface_offset(probe.motion, rel_x, rel_y))
        return (probe.origin[0] + rel_x, probe.origin[1] + rel_y, out["scale"])

    def anchor(self, point, jiggle):
        self.move_to(*point)
        direct = self.global_position()
        out = output_at(self.layout, *point) or self.layout.outputs[0]
        self.move(f"rel {inward(point[0], out, 'x') * jiggle} {inward(point[1], out, 'y') * jiggle}")
        original = self.global_position()
        recorded = (self.receiver.last().x, self.receiver.last().y)
        self.move_to(self.layout.width // 2 + 37, self.layout.height // 3)
        self.move_to(*recorded)
        landed = self.global_position()
        script = (self.receiver.last().x, self.receiver.last().y)
        return {"anchor": point, "direct": direct, "original": original, "recorded": recorded, "landed": landed, "script": script}


def surface_offset(motion, rel_x, rel_y):
    """How far the probe's own surface-local position is from QueryPointer's."""
    if motion is None:
        return float("inf")
    return max(abs(motion[0] - rel_x), abs(motion[1] - rel_y))


def inward(value, out, axis):
    start, size = (out["x"], out["w"]) if axis == "x" else (out["y"], out["h"])
    return 1 if value < start + size / 2 else -1


def errors(rows, a, b):
    """Max-axis distance between two positions, logical and physical px."""
    logical, physical = [], []
    for row in rows:
        first, second = row[a], row[b]
        if first is None or second is None:
            continue
        pa, pb = first[:2], second[:2]
        scale = second[2] if len(second) > 2 else first[2]
        d = max(abs(pa[0] - pb[0]), abs(pa[1] - pb[1]))
        logical.append(d)
        physical.append(d * scale)
    return logical, physical


COMPARISONS = [
    ("move to anchor pixel -> landed", "anchor_pos", "direct"),
    ("replay recorded -> landed", "recorded_pos", "landed"),
    ("original (sub-pixel) -> replayed", "original", "landed"),
]


def report(layout, rows):
    print(f"layout {layout.width}x{layout.height}: {layout.outputs}")
    for row in rows:
        row["anchor_pos"] = (*row["anchor"], 1.0)
        row["recorded_pos"] = (*row["recorded"], 1.0)
        row["script_pos"] = (*row["script"], 1.0)
    for title, a, b in COMPARISONS:
        logical, physical = errors(rows, a, b)
        print(f"  {title}: logical {summary(logical, 'px')}")
        print(f"  {' ' * len(title)}  physical {summary(physical, 'px')}")
    report_outliers(rows)


def distance(row, a, b):
    found = errors([row], a, b)[0]
    return found[0] if found else 0.0


def report_outliers(rows):
    for row in rows:
        if distance(row, "anchor_pos", "direct") > 0.5:
            print(f"  direct move off: anchor {row['anchor']} landed {row['direct']}")
    unseen = [row["anchor"] for row in rows if row["landed"] is None or row["direct"] is None]
    print(f"  no probe under the pointer at: {unseen}")
    disagree = [row for row in rows if row["landed"] and distance(row, "script_pos", "landed") > 0.5001]
    print(f"  QueryPointer vs script disagree (>0.5 px): {len(disagree)}")
    for row in sorted(rows, key=lambda r: -distance(r, "recorded_pos", "landed"))[:3]:
        print(f"  worst: anchor {row['anchor']} recorded {row['recorded']} landed {row['landed']} original {row['original']}")
