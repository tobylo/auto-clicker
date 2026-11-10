"""Counts the pointer events a probe window received during InputCapture.

  python3 capture_window.py PREFIX

Reads PREFIX.spike (the `capture` command's output) for each capture's start
("activated") and end ("released capture"), both stamped in milliseconds on
CLOCK_MONOTONIC, and PREFIX.wayland for the window's wl_pointer motion and
button events, whose time argument is on the same clock. Prints how many of
those events fell inside a capture (from activation up to, not including,
the millisecond input was handed back) and how many outside.
"""

import re
import sys

STAMP = r"^\[(\d+)\.\d+\] "
ACTIVATED = re.compile(STAMP + "activated")
RELEASED = re.compile(STAMP + "released capture")
POINTER = re.compile(r"wl_pointer[#@]\d+\.(?:motion|button)\((?:\d+, )?(\d+),")


def stamps(pattern, text):
    return [int(m) for m in pattern.findall(text)]


def captures(spike_log):
    with open(spike_log, encoding="utf-8") as log:
        text = log.read()
    starts = stamps(re.compile(ACTIVATED.pattern, re.M), text)
    ends = stamps(re.compile(RELEASED.pattern, re.M), text)
    return list(zip(starts, ends))


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
