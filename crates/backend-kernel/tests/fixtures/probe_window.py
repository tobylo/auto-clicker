"""A fullscreen GTK 4 window that reports the input it receives, one line each.

Used by the hand-run device test in tests/device.rs. It prints:
  ready              once the window is active
  motion X Y         pointer position in window coordinates
  press N / release N  mouse button N (1 left, 2 middle, 3 right)
  scroll DY          vertical wheel steps, positive down
  key C              a key press, as its Linux key code (layout independent)
"""

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, Gtk  # noqa: E402


def out(*words):
    print(*words, flush=True)


def build(app):
    win = Gtk.ApplicationWindow(application=app, title="auto-clicker device test")
    label = Gtk.Label(label="auto-clicker device test: leave the mouse and keyboard alone")
    label.set_vexpand(True)
    win.set_child(label)

    click = Gtk.GestureClick(button=0)
    click.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
    click.connect("pressed", lambda g, n, x, y: out("press", g.get_current_button()))
    click.connect("released", lambda g, n, x, y: out("release", g.get_current_button()))
    win.add_controller(click)

    motion = Gtk.EventControllerMotion()
    motion.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
    motion.connect("motion", lambda c, x, y: out("motion", round(x), round(y)))
    win.add_controller(motion)

    scroll = Gtk.EventControllerScroll(flags=Gtk.EventControllerScrollFlags.VERTICAL)
    scroll.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
    scroll.connect("scroll", lambda c, dx, dy: out("scroll", round(dy)) or True)
    win.add_controller(scroll)

    # XKB key codes are the Linux key codes plus 8.
    keys = Gtk.EventControllerKey()
    keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
    keys.connect("key-pressed", lambda c, val, code, state: out("key", code - 8) or True)
    win.add_controller(keys)

    ready = []

    def on_active(window, _param):
        if window.is_active() and not ready:
            ready.append(True)
            out("ready")

    win.connect("notify::is-active", on_active)
    win.fullscreen()
    win.present()


app = Gtk.Application(
    application_id="io.github.tobylo.AutoClickerDeviceTest",
    flags=Gio.ApplicationFlags.NON_UNIQUE,
)
app.connect("activate", build)
app.run(None)
