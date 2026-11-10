"""Runs the probe window with WAYLAND_DEBUG=1 and writes its two logs, with
key codes removed, so nothing typed into the window reaches the disk.

  python3 probe_logged.py PREFIX

Writes PREFIX.window (the window's own lines) and PREFIX.wayland (its raw
Wayland events). Runs until the window exits or this process gets SIGTERM,
which closes the window. Both logs are opened before the window starts, so
a log that cannot be opened stops the run before any input; draining them at
the end is bounded. Exits non-zero when a log could not be written.
"""

import os
import re
import signal
import subprocess
import sys
import threading
from pathlib import Path

PROBE = Path(__file__).resolve().parent / "probe_window.py"
# `key 30` -> `key`
WINDOW_KEY = (re.compile(r"^key \d+"), "key")
# wl_keyboard.key(serial, time, 30, 1) -> (serial, time, ?, 1)
WAYLAND_KEY = (re.compile(r"(wl_keyboard[#@]\d+\.key\(\d+, \d+, )\d+"), r"\1?")


DRAIN_LIMIT = 10.0


def pump(stream, out, rule, failures):
    """Copies `stream` to the open log `out` line by line, rewriting with `rule`."""
    pattern, replacement = rule
    try:
        with out:
            for line in stream:
                out.write(pattern.sub(replacement, line))
                out.flush()
    except OSError as err:
        failures.append(f"{out.name}: {err}")


def main(prefix):
    try:
        window_log = open(f"{prefix}.window", "w", encoding="utf-8")
        wayland_log = open(f"{prefix}.wayland", "w", encoding="utf-8")
    except OSError as err:
        sys.exit(f"log not opened: {err}")
    env = dict(os.environ, WAYLAND_DEBUG="1")
    window = subprocess.Popen(
        [sys.executable, str(PROBE)],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        errors="replace",
    )
    signal.signal(signal.SIGTERM, lambda *_: window.terminate())
    failures = []
    pumps = [
        threading.Thread(target=pump, args=(window.stdout, window_log, WINDOW_KEY, failures), daemon=True),
        threading.Thread(target=pump, args=(window.stderr, wayland_log, WAYLAND_KEY, failures), daemon=True),
    ]
    for thread in pumps:
        thread.start()
    window.wait()
    for thread in pumps:
        thread.join(timeout=DRAIN_LIMIT)
    if any(thread.is_alive() for thread in pumps):
        failures.append(f"a log was still being written after {DRAIN_LIMIT} s")
    if failures:
        sys.exit("log not written: " + "; ".join(failures))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
