#!/usr/bin/env python3
"""Cuts a MangoHud frame log into cycle.py's phases and compares frame
times with nothing shown, with the countdown and with the badge.

    frames.py MANGOHUD_CSV CYCLE_LOG [CYCLE_LOG ...]

MangoHud names its log after the second logging began (local time) and
stamps each frame with nanoseconds since then, so frames line up with the
cycle's Unix times to within a second; the first second of each phase is
left out to cover that.
"""

import csv
import datetime
import os
import re
import statistics
import sys

SETTLE = 1.0


def log_start(path):
    stamp = re.search(r"(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})", os.path.basename(path)).group(1)
    return datetime.datetime.strptime(stamp, "%Y-%m-%d_%H-%M-%S").timestamp()


def frames(path):
    start = log_start(path)
    with open(path) as handle:
        rows = list(csv.reader(handle))
    header = next(i for i, row in enumerate(rows) if row and row[0] == "fps")
    names = rows[header]
    at, time_col = names.index("elapsed"), names.index("frametime")
    return [(start + int(row[at]) / 1e9, float(row[time_col])) for row in rows[header + 1:] if len(row) == len(names)]


def phases(paths):
    spans = []
    for path in paths:
        marks = [(float(m.group(1)), m.group(2)) for m in re.finditer(r"unix (\S+) phase (.+)", open(path).read())]
        kinds = [(at, "number" if name.startswith("number") else name) for at, name in marks]
        # The countdown's three steps make one span.
        merged = [mark for index, mark in enumerate(kinds) if index == 0 or mark[1] != kinds[index - 1][1]]
        for (begin, kind), (end, _) in zip(merged, merged[1:]):
            spans.append((begin + SETTLE, end, kind))
    return spans


def summary(times):
    ordered = sorted(times)
    p99 = ordered[min(len(ordered) - 1, int(len(ordered) * 0.99))]
    return (f"frames {len(times)}, mean {statistics.mean(times):.2f} ms, median {statistics.median(times):.2f}, "
            f"p99 {p99:.2f}, max {max(times):.2f}, fps {1000 / statistics.mean(times):.1f}")


def main():
    log = frames(sys.argv[1])
    spans = phases(sys.argv[2:])
    for kind in ("none", "number", "badge"):
        times = [ft for t, ft in log for begin, end, k in spans if k == kind and begin <= t < end]
        print(f"{kind:7} {summary(times) if times else 'no frames'}")


if __name__ == "__main__":
    main()
