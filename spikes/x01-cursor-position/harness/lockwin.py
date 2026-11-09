#!/usr/bin/env python3
"""A fullscreen SDL3 window in relative mouse mode, which locks the pointer
through Wayland pointer constraints, as games do.

Prints "ready" once relative mode is on, then "motion N DX DY" (events and
summed motion so far) every 0.2 s. Closes on stdin EOF.
"""

import ctypes
import os
import select
import struct
import sys
import time

SDL_INIT_VIDEO = 0x20
SDL_WINDOW_FULLSCREEN = 0x1
SDL_EVENT_MOUSE_MOTION = 0x400

sdl = ctypes.CDLL("libSDL3.so.0")
sdl.SDL_CreateWindow.restype = ctypes.c_void_p
sdl.SDL_CreateWindow.argtypes = [ctypes.c_char_p, ctypes.c_int, ctypes.c_int, ctypes.c_uint64]
sdl.SDL_SetWindowRelativeMouseMode.argtypes = [ctypes.c_void_p, ctypes.c_bool]
sdl.SDL_SetWindowRelativeMouseMode.restype = ctypes.c_bool
sdl.SDL_GetWindowRelativeMouseMode.argtypes = [ctypes.c_void_p]
sdl.SDL_GetWindowRelativeMouseMode.restype = ctypes.c_bool
sdl.SDL_DestroyWindow.argtypes = [ctypes.c_void_p]
sdl.SDL_GetWindowFlags.argtypes = [ctypes.c_void_p]
sdl.SDL_GetWindowFlags.restype = ctypes.c_uint64
sdl.SDL_GetWindowSurface.argtypes = [ctypes.c_void_p]
sdl.SDL_GetWindowSurface.restype = ctypes.c_void_p
sdl.SDL_FillSurfaceRect.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32]
sdl.SDL_UpdateWindowSurface.argtypes = [ctypes.c_void_p]
SDL_WINDOW_INPUT_FOCUS = 0x200
SDL_WINDOW_MOUSE_FOCUS = 0x400


def stdin_closed():
    readable, _, _ = select.select([sys.stdin], [], [], 0)
    return bool(readable) and not os.read(sys.stdin.fileno(), 1)


def draw(window):
    """A Wayland window is only mapped once it commits a buffer."""
    sdl.SDL_FillSurfaceRect(sdl.SDL_GetWindowSurface(window), None, 0x202020)
    sdl.SDL_UpdateWindowSurface(window)


def drain(event, totals):
    while sdl.SDL_PollEvent(event):
        kind = struct.unpack_from("I", event, 0)[0]
        if kind == SDL_EVENT_MOUSE_MOTION:
            dx, dy = struct.unpack_from("ff", event, 36)
            totals[0] += 1
            totals[1] += dx
            totals[2] += dy


def main():
    os.environ.setdefault("SDL_VIDEO_DRIVER", "wayland")
    if not sdl.SDL_Init(SDL_INIT_VIDEO):
        return 1
    window = sdl.SDL_CreateWindow(b"x01 pointer lock", 800, 600, SDL_WINDOW_FULLSCREEN)
    event = ctypes.create_string_buffer(128)
    totals = [0, 0.0, 0.0]
    for _ in range(50):
        draw(window)
        drain(event, totals)
        time.sleep(0.02)
    sdl.SDL_SetWindowRelativeMouseMode(window, True)
    flags = sdl.SDL_GetWindowFlags(window)
    focus = f"input-focus {bool(flags & SDL_WINDOW_INPUT_FOCUS)} mouse-focus {bool(flags & SDL_WINDOW_MOUSE_FOCUS)}"
    print("ready relative", bool(sdl.SDL_GetWindowRelativeMouseMode(window)), focus, flush=True)
    while not stdin_closed():
        draw(window)
        drain(event, totals)
        print(f"motion {totals[0]} {totals[1]:.1f} {totals[2]:.1f}", flush=True)
        time.sleep(0.2)
    sdl.SDL_SetWindowRelativeMouseMode(window, False)
    sdl.SDL_DestroyWindow(window)
    sdl.SDL_Quit()
    return 0


if __name__ == "__main__":
    sys.exit(main())
