"""Counts the pointer events a probe window received during InputCapture.

  python3 capture_window.py PREFIX

Reads PREFIX.spike (the `capture` command's output) for each capture's start
("activated") and end ("released capture"), both stamped in milliseconds on
CLOCK_MONOTONIC, and PREFIX.wayland for the window's wl_pointer motion and
button events, whose time argument is on the same clock. Prints how many of
those events fell inside a capture (from activation up to, not including,
the millisecond input was handed back) and how many outside. Wayland's
times are 32-bit milliseconds, which wrap after 49.7 days of uptime; the
comparison assumes a machine up for less than that.
"""

import re
import sys

STAMP = r"^\[(\d+)\.\d+\] "
ACTIVATED = re.compile(STAMP + "activated")
RELEASED = re.compile(STAMP + "released capture: Ok")
POINTER = re.compile(r"wl_pointer[#@]\d+\.(?:motion|button)\((?:\d+, )?(\d+),")


def stamps(pattern, text):
    return [int(m) for m in pattern.findall(text)]


def captures(spike_log):
    """Each capture's (start, end). Exits when a capture has no successful
    hand-back, since its events could not be placed."""
    with open(spike_log, encoding="utf-8") as log:
        text = log.read()
    starts = stamps(re.compile(ACTIVATED.pattern, re.M), text)
    ends = stamps(re.compile(RELEASED.pattern, re.M), text)
    spans = list(zip(starts, ends))
    if len(starts) != len(ends) or any(end < start for start, end in spans):
        sys.exit(f"inconclusive: {len(starts)} activations, {len(ends)} successful hand-backs")
    return spans


def pointer_times(wayland_log):
    with open(wayland_log, encoding="utf-8", errors="replace") as log:
        for line in log:
            match = POINTER.search(line)
            if match and "->" not in line:
                yield int(match.group(1))


def main(prefix):
    spans = captures(prefix + ".spike")
    inside = outside = 0
    for time in pointer_times(prefix + ".wayland"):
        if any(start <= time < end for start, end in spans):
            inside += 1
        else:
            outside += 1
    print(f"captures (ms): {spans}")
    print(f"window pointer events during capture: {inside}, outside: {outside}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
