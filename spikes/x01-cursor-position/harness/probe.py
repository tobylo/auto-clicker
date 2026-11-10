#!/usr/bin/env python3
"""A fullscreen GTK 4 window on one monitor, so that QueryPointer has a known
window under the pointer whose origin is the output's origin.

Usage: probe.py MONITOR_INDEX. Prints "ready X Y" (the monitor's logical
origin) once shown, then "motion X Y"
in surface-local logical px for each pointer move; closes on stdin EOF.
"""

import sys

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: E402


def build(app, index):
    win = Gtk.ApplicationWindow(application=app, title=f"x01 probe {index}")
    win.set_child(Gtk.Label(label="auto-clicker X01 spike: leave the mouse alone"))
    motion = Gtk.EventControllerMotion()
    motion.connect("motion", lambda _c, x, y: print(f"motion {x!r} {y!r}", flush=True))
    win.add_controller(motion)
    monitor = Gdk.Display.get_default().get_monitors().get_item(index)
    win.fullscreen_on_monitor(monitor)
    win.present()
    origin = monitor.get_geometry()
    GLib.timeout_add(1500, lambda: print(f"ready {origin.x} {origin.y}", flush=True) and False)
    GLib.io_add_watch(sys.stdin.fileno(), GLib.IO_IN | GLib.IO_HUP, lambda *_: app.quit())


def main():
    index = int(sys.argv[1])
    app = Gtk.Application(application_id=f"io.github.tobylo.AutoClickerX01Probe{index}", flags=Gio.ApplicationFlags.NON_UNIQUE)
    app.connect("activate", lambda a: build(a, index))
    return app.run([])


if __name__ == "__main__":
    sys.exit(main())
