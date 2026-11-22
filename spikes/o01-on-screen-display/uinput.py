"""Virtual devices on /dev/uinput for the O01a runs: a mouse with relative
and absolute axes and two buttons (shaped like C01's, as decision 0004's
runs used), and a keyboard with one harmless key, right Shift."""

import fcntl
import os
import struct
import time

UI_SET_EVBIT, UI_SET_KEYBIT = 0x40045564, 0x40045565
UI_SET_RELBIT, UI_SET_ABSBIT = 0x40045566, 0x40045567
UI_DEV_SETUP, UI_ABS_SETUP = 0x405C5503, 0x401C5504
UI_DEV_CREATE, UI_DEV_DESTROY = 0x5501, 0x5502
EV_SYN, EV_KEY, EV_REL, EV_ABS = 0, 1, 2, 3
REL_X, REL_Y, ABS_X, ABS_Y = 0, 1, 0, 1
BTN_LEFT, BTN_RIGHT, KEY_RIGHTSHIFT = 0x110, 0x111, 54
ABS_MAX = 65535


def _create(name, product, setup):
    fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
    setup(fd)
    fcntl.ioctl(fd, UI_DEV_SETUP, struct.pack("HHHH80sI", 3, 0x1234, product, 1, name, 0))
    fcntl.ioctl(fd, UI_DEV_CREATE)
    return fd


def _mouse_bits(fd):
    fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
    for button in (BTN_LEFT, BTN_RIGHT):
        fcntl.ioctl(fd, UI_SET_KEYBIT, button)
    fcntl.ioctl(fd, UI_SET_EVBIT, EV_REL)
    for axis in (REL_X, REL_Y):
        fcntl.ioctl(fd, UI_SET_RELBIT, axis)
    fcntl.ioctl(fd, UI_SET_EVBIT, EV_ABS)
    for axis in (ABS_X, ABS_Y):
        fcntl.ioctl(fd, UI_SET_ABSBIT, axis)
        fcntl.ioctl(fd, UI_ABS_SETUP, struct.pack("HH6i", axis, 0, 0, 0, ABS_MAX, 0, 0, 0))


def _keyboard_bits(fd):
    fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
    fcntl.ioctl(fd, UI_SET_KEYBIT, KEY_RIGHTSHIFT)


class Devices:
    """The mouse and the keyboard; `close` releases everything and destroys
    both, whatever state the run left them in."""

    def __init__(self):
        self.mouse = _create(b"o01a-probe-mouse", 0x5701, _mouse_bits)
        self.keyboard = _create(b"o01a-probe-keyboard", 0x5702, _keyboard_bits)

    @staticmethod
    def _emit(fd, *events):
        for kind, code, value in events:
            os.write(fd, struct.pack("llHHi", 0, 0, kind, code, value))
        os.write(fd, struct.pack("llHHi", 0, 0, EV_SYN, 0, 0))

    def move(self, dx, dy):
        self._emit(self.mouse, (EV_REL, REL_X, dx), (EV_REL, REL_Y, dy))

    def place(self, x, y, box):
        """Puts the pointer at logical (x, y) of the layout's bounding box
        `box` = (left, top, width, height), decision 0001's formula."""
        left, top, width, height = box
        value = lambda at, start, size: max(0, min(ABS_MAX, round((at - start) * 65536 / size)))
        self._emit(self.mouse, (EV_ABS, ABS_X, value(x, left, width)), (EV_ABS, ABS_Y, value(y, top, height)))

    def button(self, pressed, code=BTN_LEFT):
        self._emit(self.mouse, (EV_KEY, code, 1 if pressed else 0))

    def key(self, pressed):
        self._emit(self.keyboard, (EV_KEY, KEY_RIGHTSHIFT, 1 if pressed else 0))

    def close(self):
        for fd, events in ((self.mouse, ((EV_KEY, BTN_LEFT, 0), (EV_KEY, BTN_RIGHT, 0))),
                           (self.keyboard, ((EV_KEY, KEY_RIGHTSHIFT, 0),))):
            try:
                self._emit(fd, *events)
                time.sleep(0.05)
                fcntl.ioctl(fd, UI_DEV_DESTROY)
            finally:
                os.close(fd)
