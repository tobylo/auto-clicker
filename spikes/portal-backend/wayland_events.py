"""Summarises the input a probe window received, from its WAYLAND_DEBUG log.

  python3 wayland_events.py PREFIX.wayland

Prints each pointer button and keyboard key event with the compositor's
millisecond time, then the press count and the periods between left presses.
Key events show only whether a key went down or up, not which key.
"""

import re
import sys

BUTTON = re.compile(r"wl_pointer[#@]\d+\.button\(\d+, (\d+), (\d+), (\d+)\)")
KEY = re.compile(r"wl_keyboard[#@]\d+\.key\(\d+, (\d+), (?:\d+|\?), (\d+)\)")
OTHER = re.compile(r"wl_pointer[#@]\d+\.(enter|leave|axis_value120|axis_discrete)\(")
BTN_LEFT = 272
STATES = {"0": "up", "1": "down", "2": "repeat"}


def events(path):
    with open(path, encoding="utf-8", errors="replace") as log:
        for text in log:
            if text.lstrip().startswith("->"):
                continue
            if match := BUTTON.search(text):
                time, button, state = match.groups()
                yield int(time), f"button {int(button):#x} {STATES.get(state, state)}"
            elif match := KEY.search(text):
                time, state = match.groups()
                yield int(time), f"key {STATES.get(state, state)}"
            elif match := OTHER.search(text):
                yield None, f"pointer {match.group(1)}"


def periods(lines):
    presses = [t for t, what in lines if what == f"button {BTN_LEFT:#x} down"]
    return presses, [b - a for a, b in zip(presses, presses[1:])]


def shown(what, many):
    """Key repeats are only counted, and buttons are not listed in long runs."""
    if what == "key repeat":
        return False
    return not (many and what.startswith("button"))


def main(path):
    lines = list(events(path))
    repeats = sum(1 for _, what in lines if what == "key repeat")
    for time, what in lines:
        if shown(what, len(lines) > 200):
            print(time, what)
    if repeats:
        print(f"key repeats sent by the compositor: {repeats}")
    presses, gaps = periods(lines)
    print(f"left presses: {len(presses)}")
    if gaps:
        gaps.sort()
        print(f"period ms: min {gaps[0]}, median {gaps[len(gaps) // 2]}, max {gaps[-1]}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
