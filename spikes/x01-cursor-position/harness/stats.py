"""Small helpers for summarising measurements."""

import os


def percentile(values, fraction):
    ordered = sorted(values)
    if not ordered:
        return float("nan")
    index = min(len(ordered) - 1, max(0, round(fraction * (len(ordered) - 1))))
    return ordered[index]


def summary(values, unit):
    if not values:
        return "no values"
    parts = [f"p{int(f * 100)} {percentile(values, f):.3f}" for f in (0.5, 0.9, 0.99)]
    return f"n {len(values)}  " + "  ".join(parts) + f"  max {max(values):.3f} {unit}"


def cpu_ticks(pid):
    """utime + stime of a process, in clock ticks."""
    with open(f"/proc/{pid}/stat") as stat:
        fields = stat.read().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])


def pids(name):
    """The user's processes whose command name is exactly name."""
    found = []
    uid = os.getuid()
    for entry in os.listdir("/proc"):
        if entry.isdigit() and _owned_named(entry, uid, name):
            found.append(int(entry))
    return found


def _owned_named(pid, uid, name):
    try:
        with open(f"/proc/{pid}/comm") as comm:
            if comm.read().strip() != name:
                return False
        return os.stat(f"/proc/{pid}").st_uid == uid
    except OSError:
        return False


TICK = os.sysconf("SC_CLK_TCK")
