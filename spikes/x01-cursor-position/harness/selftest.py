#!/usr/bin/env python3
"""Self-test of the harness's own guards; needs no Plasma session.

Run: dbus-run-session -- python3 harness/selftest.py

- The receiver accepts calls only from org.kde.KWin's current owner.
- wait_ready is bounded, reaps a helper that fails, and leaves the output
  after the readiness line to the caller.
"""

import subprocess
import sys
import time

import dbus

import bus
from children import wait_ready

HELPERS = {
    "ready": ('print("ready", flush=True); print("motion 1 2", flush=True)', "ready"),
    "partial": ('import sys; sys.stdout.write("ready"); sys.stdout.flush()', None),
    "bad bytes": ('import sys; sys.stdout.buffer.write(b"\\xff\\n"); sys.stdout.flush()', None),
    "silent": ("pass", None),
}
SLEEP = "; import time; time.sleep(30)"
# Well under the helpers' 30 s lifetime, above the 0.5 s deadline plus
# stop()'s 5 s grace: a reader that ignored its deadline would take 30 s.
BOUND = 15


def check(condition, what):
    print(("ok   " if condition else "FAIL ") + what)
    return condition


def call(conn, method, *args):
    conn.call_async(bus.SERVICE, "/", bus.SERVICE, method, None, args, lambda *_: None, lambda e: print("error", e))


def receiver_checks():
    kwin = dbus.bus.BusConnection(dbus.bus.BUS_SESSION)
    kwin.request_name("org.kde.KWin")
    stranger = dbus.bus.BusConnection(dbus.bus.BUS_SESSION)
    receiver = bus.Receiver(bus.session_bus())
    call(kwin, "Position", dbus.Int32(5), dbus.Int32(6), "1 100")
    call(stranger, "Position", dbus.Int32(9), dbus.Int32(9), "2 101")
    bus.pump_for(0.3)
    results = [check([(s.x, s.y) for s in receiver.samples] == [(5, 6)], "KWin accepted, stranger rejected")]
    successor = dbus.bus.BusConnection(dbus.bus.BUS_SESSION)
    kwin.release_name("org.kde.KWin")
    successor.request_name("org.kde.KWin")
    bus.pump_for(0.3)
    call(kwin, "Position", dbus.Int32(7), dbus.Int32(7), "3 102")
    call(successor, "Position", dbus.Int32(8), dbus.Int32(8), "4 103")
    bus.pump_for(0.3)
    results.append(check(receiver.samples[-1].x == 8 and len(receiver.samples) == 2, "former KWin owner rejected"))
    return all(results)


def helper_check(name, code, expected):
    proc = subprocess.Popen([sys.executable, "-c", code + SLEEP], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    start = time.monotonic()
    try:
        line = wait_ready(proc, timeout=0.5)
    except RuntimeError:
        line = None
    took = time.monotonic() - start
    if expected:
        ok = line == expected and proc.stdout.readline() == b"motion 1 2\n"
    else:
        ok = line is None and proc.poll() is not None and took < BOUND
    if proc.poll() is None:
        proc.kill()
        proc.wait()
    return check(ok, f"wait_ready: {name}")


def main():
    results = [receiver_checks()]
    results += [helper_check(name, code, expected) for name, (code, expected) in HELPERS.items()]
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
