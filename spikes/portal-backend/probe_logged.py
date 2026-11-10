"""Runs the probe window with WAYLAND_DEBUG=1 and writes its two logs, with
key codes removed, so nothing typed into the window reaches the disk.

  python3 probe_logged.py PREFIX

Writes PREFIX.window (the window's own lines) and PREFIX.wayland (its raw
Wayland events). Runs until the window exits or this process gets SIGTERM,
which closes the window. Exits non-zero when a log could not be written.
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


def pump(stream, path, rule, failures):
    """Copies `stream` to `path` line by line, rewriting with `rule`."""
    pattern, replacement = rule
    try:
        with open(path, "w", encoding="utf-8") as out:
            for line in stream:
                out.write(pattern.sub(replacement, line))
                out.flush()
    except OSError as err:
        failures.append(f"{path}: {err}")


def main(prefix):
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
        threading.Thread(target=pump, args=(window.stdout, f"{prefix}.window", WINDOW_KEY, failures)),
        threading.Thread(target=pump, args=(window.stderr, f"{prefix}.wayland", WAYLAND_KEY, failures)),
    ]
    for thread in pumps:
        thread.start()
    window.wait()
    for thread in pumps:
        thread.join()
    if failures:
        sys.exit("log not written: " + "; ".join(failures))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
