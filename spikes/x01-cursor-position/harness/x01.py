#!/usr/bin/env python3
"""X01 spike: read the cursor position on KWin through a script.

Run every subcommand that moves the pointer under the shared spike lock:
  flock /tmp/auto-clicker-spike.lock python3 harness/x01.py <subcommand> ...
"""

import argparse
import sys

from bus import Receiver, exit_on_sigterm, loaded_script, pump_for, session_bus


def layout(_args):
    """Prints what the script reports about the layout, then unloads it."""
    bus = session_bus()
    receiver = Receiver(bus)
    with loaded_script(bus):
        pump_for(1.0)
    for item in receiver.layouts:
        print(f"layout {item.width}x{item.height} at {item.origin}: {item.outputs}")


def receive(args):
    """Loads the script and keeps receiving; for the crash check."""
    bus = session_bus()
    receiver = Receiver(bus)
    with loaded_script(bus):
        print("receiving", flush=True)
        pump_for(args.seconds)
    print(f"received {len(receiver.samples)}")


def latency(args):
    import timing

    timing.latency(args.hz, args.sweeps, args.count)


def cpu(args):
    import timing

    timing.cpu(args.hz, args.seconds, args.mode)


def mapping(args):
    import mapping as check

    check.run(args.monitors, args.jiggle)


def lock(_args):
    import pointer_lock

    pointer_lock.run()


def parser():
    top = argparse.ArgumentParser(description=__doc__)
    sub = top.add_subparsers(required=True)
    sub.add_parser("layout").set_defaults(run=layout)
    p = sub.add_parser("receive")
    p.add_argument("--seconds", type=float, default=60)
    p.set_defaults(run=receive)
    p = sub.add_parser("latency")
    p.add_argument("--hz", type=int, default=1000)
    p.add_argument("--sweeps", type=int, default=5)
    p.add_argument("--count", type=int, default=900)
    p.set_defaults(run=latency)
    p = sub.add_parser("cpu")
    p.add_argument("--hz", type=int, default=1000)
    p.add_argument("--seconds", type=int, default=10)
    p.add_argument("--mode", choices=["on", "off", "dead"], default="on")
    p.set_defaults(run=cpu)
    p = sub.add_parser("mapping")
    p.add_argument("--monitors", type=int, default=1)
    p.add_argument("--jiggle", type=int, default=7)
    p.set_defaults(run=mapping)
    sub.add_parser("lock").set_defaults(run=lock)
    return top


def main():
    exit_on_sigterm()
    args = parser().parse_args()
    args.run(args)


if __name__ == "__main__":
    sys.exit(main())
