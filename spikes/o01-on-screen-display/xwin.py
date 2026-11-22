#!/usr/bin/env python3
"""A fullscreen X11 window that holds the pointer as older and Wine games
do: it grabs the pointer confined to itself, hides the cursor and warps the
pointer back to its centre after each move, summing the moves. Xwayland
turns a hidden cursor that is warped into a pointer lock (its warp
emulator), so the moves keep coming past the screen's edge only while that
lock holds. Prints in the probe's line format. Uses libX11 through ctypes.

    xwin.py locked|free
"""

import ctypes
import ctypes.util
import os
import select
import struct
import sys
import time

KEY_PRESS, BUTTON_PRESS, BUTTON_RELEASE, MOTION = 2, 4, 5, 6
CONFIGURE = 22
ENTER, LEAVE, FOCUS_IN, FOCUS_OUT, MAP = 7, 8, 9, 10, 19
MASK = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 17) | (1 << 21)
POINTER_MASK = (1 << 2) | (1 << 3) | (1 << 6)
NOTIFY_GRAB, NOTIFY_UNGRAB = 1, 2

x = ctypes.CDLL(ctypes.util.find_library("X11"))
x.XOpenDisplay.restype = ctypes.c_void_p
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
for name in ("XDefaultRootWindow", "XCreateSimpleWindow", "XInternAtom", "XCreateBitmapFromData", "XCreatePixmapCursor"):
    getattr(x, name).restype = ctypes.c_ulong
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint,
                                  ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_ulong]
x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XChangeProperty.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int,
                              ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
x.XSelectInput.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_long]
x.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XPending.argtypes = [ctypes.c_void_p]
x.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
x.XFlush.argtypes = [ctypes.c_void_p]
x.XCreateBitmapFromData.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_char_p, ctypes.c_uint, ctypes.c_uint]
x.XCreatePixmapCursor.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p, ctypes.c_void_p,
                                  ctypes.c_uint, ctypes.c_uint]
x.XDefineCursor.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong]
x.XGrabPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_uint, ctypes.c_int, ctypes.c_int,
                           ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
x.XWarpPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                           ctypes.c_uint, ctypes.c_uint, ctypes.c_int, ctypes.c_int]
x.XConnectionNumber.argtypes = [ctypes.c_void_p]
x.XNextRequest.argtypes = [ctypes.c_void_p]
x.XNextRequest.restype = ctypes.c_ulong
x.XGetGeometry.argtypes = [ctypes.c_void_p, ctypes.c_ulong] + [ctypes.c_void_p] * 7


def say(text):
    print(f"{time.monotonic():.4f} {text}", flush=True)


def open_window(display):
    root = x.XDefaultRootWindow(display)
    window = x.XCreateSimpleWindow(display, root, 0, 0, 800, 600, 0, 0, 0x202A33)
    state = x.XInternAtom(display, b"_NET_WM_STATE", 0)
    fullscreen = ctypes.c_long(x.XInternAtom(display, b"_NET_WM_STATE_FULLSCREEN", 0))
    x.XChangeProperty(display, window, state, 4, 32, 0, ctypes.byref(fullscreen), 1)
    x.XSelectInput(display, window, MASK)
    x.XMapWindow(display, window)
    return window


x.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]


def ask_fullscreen(display, window):
    """Asks the window manager for fullscreen after mapping, as EWMH says;
    xwayland-satellite ignores the state set before the window maps."""
    event = ctypes.create_string_buffer(192)
    state = x.XInternAtom(display, b"_NET_WM_STATE", 0)
    fullscreen = x.XInternAtom(display, b"_NET_WM_STATE_FULLSCREEN", 0)
    struct.pack_into("i", event, 0, 33)  # ClientMessage
    struct.pack_into("Q", event, 32, window)
    struct.pack_into("Q", event, 40, state)
    struct.pack_into("i", event, 48, 32)
    struct.pack_into("qqq", event, 56, 1, fullscreen, 0)
    redirect_and_notify = (1 << 20) | (1 << 19)
    x.XSendEvent(display, x.XDefaultRootWindow(display), 0, redirect_and_notify, event)
    x.XFlush(display)


def hide_and_grab(display, window):
    blank = x.XCreateBitmapFromData(display, window, b"\0", 1, 1)
    colour = ctypes.create_string_buffer(16)
    cursor = x.XCreatePixmapCursor(display, blank, blank, colour, colour, 0, 0)
    x.XDefineCursor(display, window, cursor)
    for _ in range(100):
        if x.XGrabPointer(display, window, 1, POINTER_MASK, 1, 1, window, cursor, 0) == 0:
            return True
        time.sleep(0.02)
    return False


def size(display, window):
    values = [ctypes.c_uint() for _ in range(4)]
    root, at_x, at_y = ctypes.c_ulong(), ctypes.c_int(), ctypes.c_int()
    x.XGetGeometry(display, window, ctypes.byref(root), ctypes.byref(at_x), ctypes.byref(at_y),
                   *[ctypes.byref(v) for v in values])
    return values[0].value, values[1].value


class Heard:
    def __init__(self, display, window, lock):
        self.display, self.window, self.lock = display, window, lock
        self.centre = (0, 0)
        # The last position heard, and the request serial of the latest warp
        # to the centre. Moves are counted from the last position, so moves
        # queued before a warp lands are each counted once. Only an event the
        # server sent after that warp (its serial at least the warp's) can be
        # the warp's own motion; a real move back to the centre queued before
        # it is still counted.
        self.last = (0, 0)
        self.warp_serial = None
        self.relative = [0, 0, 0]
        self.printed = None

    def warp_to_centre(self):
        """Warps the pointer to the centre; its motion event is not a move."""
        self.warp_serial = x.XNextRequest(self.display)
        x.XWarpPointer(self.display, 0, self.window, 0, 0, 0, 0, *self.centre)

    def report(self):
        if self.printed != tuple(self.relative):
            count, dx, dy = self.relative
            say(f"relative total {count} sum {dx},{dy}")
            self.printed = tuple(self.relative)

    def motion(self, at_x, at_y, serial):
        after_warp = self.warp_serial is not None and serial >= self.warp_serial
        if after_warp and (at_x, at_y) == self.centre:
            self.warp_serial = None
            self.last = self.centre
            return
        dx, dy = at_x - self.last[0], at_y - self.last[1]
        self.last = (at_x, at_y)
        if (dx, dy) == (0, 0):
            return
        self.relative[0] += 1
        self.relative[1] += dx
        self.relative[2] += dy
        if self.needs_warp():
            self.warp_to_centre()

    def needs_warp(self):
        """Whether to warp back to the centre now. One warp in flight at a
        time: a second would leave the first's motion looking like a real
        move back to the centre."""
        off_centre = self.last != self.centre
        return self.lock and off_centre and self.warp_serial is None

    def configured(self, kind, event):
        width, height = struct.unpack_from("ii", event, 56)
        if (width // 2, height // 2) != self.centre and self.lock:
            self.centre = (width // 2, height // 2)
            say(f"configured {width}x{height}")
            self.warp_to_centre()

    def moved(self, kind, event):
        serial = struct.unpack_from("Q", event, 8)[0]
        self.motion(*struct.unpack_from("ii", event, 64), serial)

    def button(self, kind, event):
        self.report()
        at_x, at_y = struct.unpack_from("ii", event, 64)
        button = struct.unpack_from("I", event, 84)[0]
        say(f"button {button} {'press' if kind == BUTTON_PRESS else 'release'} at {at_x},{at_y}")

    def key(self, kind, event):
        say(f"key {struct.unpack_from('I', event, 84)[0]} {'Pressed' if kind == KEY_PRESS else 'Released'}")

    def focus(self, kind, event):
        self.crossing(struct.unpack_from("i", event, 40)[0], "keyboard", kind == FOCUS_IN)

    def pointer(self, kind, event):
        self.crossing(struct.unpack_from("i", event, 80)[0], "pointer", kind == ENTER)

    def crossing(self, mode, what, gained):
        """Reports focus gained or lost, but not the moves a grab makes."""
        if mode not in (NOTIFY_GRAB, NOTIFY_UNGRAB):
            self.report()
            say(f"{what} focus: {'gained' if gained else 'LOST'}")

    def take(self, event):
        kind = struct.unpack_from("i", event, 0)[0]
        handler = HANDLERS.get(kind)
        if handler:
            handler(self, kind, event)


HANDLERS = {
    CONFIGURE: Heard.configured,
    MOTION: Heard.moved,
    BUTTON_PRESS: Heard.button,
    BUTTON_RELEASE: Heard.button,
    KEY_PRESS: Heard.key,
    KEY_PRESS + 1: Heard.key,
    FOCUS_IN: Heard.focus,
    FOCUS_OUT: Heard.focus,
    ENTER: Heard.pointer,
    LEAVE: Heard.pointer,
}


def stdin_says_quit():
    readable, _, _ = select.select([sys.stdin], [], [], 0)
    if not readable:
        return False
    data = os.read(sys.stdin.fileno(), 4096)
    return not data or b"quit" in data


class Window:
    """The window and what it heard, made fullscreen and grabbed once it maps."""

    def __init__(self, display, lock):
        self.display, self.lock = display, lock
        self.window = open_window(display)
        say(f"window asked for, driver=x11 (Xlib) lock={lock}")
        self.heard = Heard(display, self.window, lock)
        self.mapped = False

    def on_map(self):
        self.mapped = True
        ask_fullscreen(self.display, self.window)
        time.sleep(0.5)
        width, height = size(self.display, self.window)
        self.heard.centre = self.heard.last = (width // 2, height // 2)
        grabbed = hide_and_grab(self.display, self.window) if self.lock else False
        say(f"mapped {width}x{height}; grabbed={grabbed}")
        if self.lock:
            self.heard.warp_to_centre()

    def drain(self, event):
        """Handles every event already queued."""
        while x.XPending(self.display):
            x.XNextEvent(self.display, event)
            if struct.unpack_from("i", event, 0)[0] == MAP and not self.mapped:
                self.on_map()
            self.heard.take(event)
        x.XFlush(self.display)


def main():
    lock = sys.argv[1:2] != ["free"]
    display = x.XOpenDisplay(None)
    if not display:
        return 1
    window = Window(display, lock)
    event = ctypes.create_string_buffer(192)
    last = time.monotonic()
    while not stdin_says_quit():
        window.drain(event)
        if time.monotonic() - last > 0.1:
            window.heard.report()
            last = time.monotonic()
        select.select([x.XConnectionNumber(display)], [], [], 0.01)
    window.heard.report()
    say("window closed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
