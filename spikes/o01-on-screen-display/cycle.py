#!/usr/bin/env python3
"""Shows the display above a game on a timer, for the runs with a person
playing: 15 s of nothing, then four rounds of a countdown (3, 2, 1, each
1 s, under the crosshair), 3 s of nothing, the badge for 8 s and 4 s of
nothing. Each change is printed on CLOCK_MONOTONIC and in Unix time, so a
frame-time log can be cut into the phases.

    cycle.py [ROUNDS]
"""

import os
import subprocess
import sys
import time

PROBE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "target/release/o01-probe")


def mark(phase):
    print(f"{time.monotonic():.4f} unix {time.time():.4f} phase {phase}", flush=True)


def main():
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 4
    display = subprocess.Popen([PROBE, "show"], stdin=subprocess.PIPE, text=True, bufsize=1)
    steps = [("none", 15)]
    for _ in range(rounds):
        steps += [("number 3", 1), ("number 2", 1), ("number 1", 1), ("none", 3), ("badge", 8), ("none", 4)]
    try:
        for command, seconds in steps:
            display.stdin.write(command + "\n")
            display.stdin.flush()
            mark(command)
            time.sleep(seconds)
        mark("end")
    finally:
        display.stdin.close()
        try:
            display.wait(timeout=3)
        except subprocess.TimeoutExpired:
            display.kill()
            display.wait(timeout=3)


if __name__ == "__main__":
    main()
