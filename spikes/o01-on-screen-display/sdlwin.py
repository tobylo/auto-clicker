#!/usr/bin/env python3
"""A fullscreen SDL 3 window that prints what it hears, in the probe's line
format, for the runs under XWayland (SDL_VIDEO_DRIVER=x11) and as a second
native client (wayland).

    sdlwin.py locked|free

"locked" turns on relative mouse mode, which locks the pointer as games do
(on Wayland through pointer-constraints; under XWayland through an X grab
that Xwayland turns into a lock). Quits on stdin's end or "quit".
"""

import ctypes
import os
import select
import struct
import sys
import time

SDL_INIT_VIDEO = 0x20
SDL_WINDOW_FULLSCREEN = 0x1
KEY_DOWN, KEY_UP = 0x300, 0x301
MOTION, BUTTON_DOWN, BUTTON_UP = 0x400, 0x401, 0x402
WINDOW_EVENTS = {
    0x20C: "pointer focus: gained",
    0x20D: "pointer focus: LOST",
    0x20E: "keyboard focus: gained",
    0x20F: "keyboard focus: LOST",
}

sdl = ctypes.CDLL("libSDL3.so.0")
sdl.SDL_Init.argtypes = [ctypes.c_uint32]
sdl.SDL_Init.restype = ctypes.c_bool
sdl.SDL_CreateWindow.restype = ctypes.c_void_p
sdl.SDL_CreateWindow.argtypes = [ctypes.c_char_p, ctypes.c_int, ctypes.c_int, ctypes.c_uint64]
sdl.SDL_SetWindowRelativeMouseMode.argtypes = [ctypes.c_void_p, ctypes.c_bool]
sdl.SDL_SetWindowRelativeMouseMode.restype = ctypes.c_bool
sdl.SDL_PollEvent.argtypes = [ctypes.c_void_p]
sdl.SDL_PollEvent.restype = ctypes.c_bool
sdl.SDL_GetWindowSurface.argtypes = [ctypes.c_void_p]
sdl.SDL_GetWindowSurface.restype = ctypes.c_void_p
sdl.SDL_FillSurfaceRect.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32]
sdl.SDL_FillSurfaceRect.restype = ctypes.c_bool
sdl.SDL_UpdateWindowSurface.argtypes = [ctypes.c_void_p]
sdl.SDL_UpdateWindowSurface.restype = ctypes.c_bool
sdl.SDL_GetCurrentVideoDriver.restype = ctypes.c_char_p
sdl.SDL_DestroyWindow.argtypes = [ctypes.c_void_p]


def say(text):
    print(f"{time.monotonic():.4f} {text}", flush=True)


class Heard:
    """Summed motion, printed at most every 100 ms."""

    def __init__(self):
        self.relative = [0, 0.0, 0.0]
        self.position = (0.0, 0.0)
        self.printed = None

    def report(self):
        count, dx, dy = self.relative
        if self.printed != (count, dx, dy):
            say(f"relative total {count} sum {dx:.0f},{dy:.0f}")
            self.printed = (count, dx, dy)

    def take(self, event):
        kind = struct.unpack_from("I", event, 0)[0]
        if kind == MOTION:
            x, y, dx, dy = struct.unpack_from("ffff", event, 28)
            self.relative[0] += 1
            self.relative[1] += dx
            self.relative[2] += dy
            self.position = (x, y)
        elif kind in (BUTTON_DOWN, BUTTON_UP):
            button = struct.unpack_from("B", event, 24)[0]
            x, y = struct.unpack_from("ff", event, 28)
            self.report()
            what = "press" if kind == BUTTON_DOWN else "release"
            say(f"button {button} {what} at {x:.1f},{y:.1f}")
        elif kind in (KEY_DOWN, KEY_UP):
            scancode = struct.unpack_from("I", event, 24)[0]
            say(f"key {scancode} {'Pressed' if kind == KEY_DOWN else 'Released'}")
        elif kind in WINDOW_EVENTS:
            self.report()
            say(WINDOW_EVENTS[kind])


def stdin_says_quit():
    readable, _, _ = select.select([sys.stdin], [], [], 0)
    if not readable:
        return False
    data = os.read(sys.stdin.fileno(), 4096)
    return not data or b"quit" in data


def main():
    lock = sys.argv[1:2] != ["free"]
    if not sdl.SDL_Init(SDL_INIT_VIDEO):
        return 1
    window = sdl.SDL_CreateWindow(b"auto-clicker O01a SDL window", 800, 600, SDL_WINDOW_FULLSCREEN)
    if not window:
        return 1
    say(f"window asked for, driver={sdl.SDL_GetCurrentVideoDriver().decode()} lock={lock}")
    event = ctypes.create_string_buffer(128)
    heard = Heard()
    if lock:
        say(f"relative mode: {sdl.SDL_SetWindowRelativeMouseMode(window, True)}")
    frames = 0
    while not stdin_says_quit():
        sdl.SDL_FillSurfaceRect(sdl.SDL_GetWindowSurface(window), None, 0x202A33)
        sdl.SDL_UpdateWindowSurface(window)
        while sdl.SDL_PollEvent(event):
            heard.take(event)
        frames += 1
        if frames % 10 == 0:
            heard.report()
        time.sleep(0.01)
    heard.report()
    sdl.SDL_DestroyWindow(window)
    say("window closed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
