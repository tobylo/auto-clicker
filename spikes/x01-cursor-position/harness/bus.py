"""The D-Bus side of the X01 spike: the receiver, the KWin script and the loop.

The receiver owns SERVICE and takes the calls the KWin script makes. Every
wait pumps the GLib main context, so calls are timestamped as they arrive.
"""

import os
import signal
import time
from contextlib import contextmanager
from dataclasses import dataclass, field

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

SERVICE = "io.github.tobylo.AutoClickerX01"
PLUGIN = "x01-cursor"
HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(HERE, "kwin", "x01-cursor.js")

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)


@dataclass
class Sample:
    """One Position call: when it arrived, and what the script sent."""

    recv_ns: int
    recv_wall_ms: float
    x: int
    y: int
    callbacks: int
    script_ms: float


@dataclass
class Layout:
    width: int
    height: int
    origin: tuple
    outputs: list = field(default_factory=list)


class Receiver(dbus.service.Object):
    """Owns SERVICE and records every call the script makes."""

    def __init__(self, bus):
        self.samples = []
        self.layouts = []
        super().__init__(dbus.service.BusName(SERVICE, bus, do_not_queue=True), "/")

    @dbus.service.method(SERVICE)
    def Position(self, x, y, callbacks, script_ms):  # noqa: N802 (D-Bus name)
        now = time.monotonic_ns()
        wall = time.time_ns() / 1e6
        self.samples.append(Sample(now, wall, int(x), int(y), int(callbacks), float(script_ms)))

    @dbus.service.method(SERVICE)
    def Layout(self, box, outputs):  # noqa: N802 (D-Bus name)
        x, y, width, height = (int(v) for v in str(box).split())
        self.layouts.append(Layout(width, height, (x, y), parse_outputs(outputs)))

    def last(self):
        return self.samples[-1] if self.samples else None


def parse_outputs(text):
    """'name x y w h scale;...' into dicts."""
    outputs = []
    for item in str(text).split(";"):
        if item:
            name, x, y, w, h, scale = item.split()
            outputs.append({"name": name, "x": int(x), "y": int(y), "w": int(w), "h": int(h), "scale": float(scale)})
    return outputs


def session_bus():
    return dbus.SessionBus()


def scripting(bus):
    return dbus.Interface(bus.get_object("org.kde.KWin", "/Scripting"), "org.kde.kwin.Scripting")


def unload_stale(bus):
    """Unloads a script a crashed run left loaded; returns whether there was one."""
    if bool(scripting(bus).isScriptLoaded(PLUGIN)):
        scripting(bus).unloadScript(PLUGIN)
        return True
    return False


def load_script(bus):
    script_id = int(scripting(bus).loadScript(SCRIPT, PLUGIN, signature="ss"))
    if script_id < 0:
        raise RuntimeError(f"KWin refused the script: {script_id}")
    script = bus.get_object("org.kde.KWin", f"/Scripting/Script{script_id}")
    dbus.Interface(script, "org.kde.kwin.Script").run()
    return script_id


@contextmanager
def loaded_script(bus):
    """Loads the script for the block and always unloads it afterwards."""
    stale = unload_stale(bus)
    if stale:
        print("unloaded a stale script left by an earlier run")
    load_script(bus)
    try:
        yield
    finally:
        scripting(bus).unloadScript(PLUGIN)


def exit_on_sigterm():
    """SIGTERM and SIGHUP raise SystemExit, so `finally` blocks run."""

    def stop(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGHUP, stop)


def pump_until(done, timeout):
    """Runs the main loop until done() holds or timeout seconds pass."""
    expired = []
    source = GLib.timeout_add(max(1, int(timeout * 1000)), lambda: expired.append(True) and False)
    context = GLib.MainContext.default()
    while not done() and not expired:
        context.iteration(True)
    if not expired:
        GLib.source_remove(source)
    return done()


def pump_for(seconds):
    pump_until(lambda: False, seconds)


def query_pointer(bus):
    """org.freedesktop.a11y.PointerLocator.QueryPointer: (pid, rel_x, rel_y)."""
    manager = bus.get_object("org.freedesktop.a11y.Manager", "/org/freedesktop/a11y/Manager")
    data, rel_x, rel_y = dbus.Interface(manager, "org.freedesktop.a11y.PointerLocator").QueryPointer()
    return int(data.get("pid", -1)), float(rel_x), float(rel_y)
