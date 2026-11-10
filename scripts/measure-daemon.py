#!/usr/bin/env python3
"""Measures the idle daemon, by hand, in a graphical session.

  measure-daemon.py [SECONDS]   idle cost over SECONDS (default 60)

Starts the release build (build it first with `cargo build --release --locked
-p auto-clicker`), waits for it to settle, and samples its CPU time, context
switches, threads and resident memory before and after the idle period. Then
it sends SIGTERM and checks the daemon exits with 0. Needs the daemon's usual
access (/dev/uinput, and /dev/input for hotkeys), a session bus and no other
running instance. Needs `setpriv` from util-linux. Leave the hotkeys alone
while it runs.
"""

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DAEMON = ROOT / "target/release/auto-clicker"
SETTLE = 3.0
EXIT_TIMEOUT = 5.0


def sample(pid):
    """CPU ticks (user + system), context switches by thread id and the
    resident set in KiB. Process CPU time includes threads that have exited;
    their context switches are gone with them, so switches are compared per
    thread."""
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])
    switches = {task.name: context_switches(task) for task in Path(f"/proc/{pid}/task").iterdir()}
    return ticks, switches, resident_kib(pid)


def new_switches(before, after):
    """Context switches made between two samples by the threads alive at the
    second; a thread started in between counts from zero."""
    return sum(count - before.get(tid, 0) for tid, count in after.items())


def context_switches(task):
    lines = (task / "status").read_text().splitlines()
    counts = [line.split()[1] for line in lines if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"))]
    return sum(int(count) for count in counts)


def resident_kib(pid):
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    sys.exit("VmRSS missing from /proc status")


def terminate(daemon):
    """Sends SIGTERM and returns the exit status; kills a daemon that does
    not exit in time, so none is left behind."""
    daemon.send_signal(signal.SIGTERM)
    try:
        return daemon.wait(timeout=EXIT_TIMEOUT)
    except subprocess.TimeoutExpired:
        daemon.kill()
        daemon.wait()
        sys.exit("the daemon did not exit after SIGTERM")


def main(argv):
    seconds = int(argv[1]) if len(argv) > 1 else 60
    if not DAEMON.exists():
        sys.exit(f"{DAEMON} is missing; build it first")
    # The kernel sends the daemon SIGTERM if this script dies first.
    daemon = subprocess.Popen(["setpriv", "--pdeathsig", "TERM", "--", str(DAEMON)])
    try:
        time.sleep(SETTLE)
        if daemon.poll() is not None:
            sys.exit(f"the daemon exited at start ({daemon.returncode})")
        ticks0, switches0, _ = sample(daemon.pid)
        time.sleep(seconds)
        ticks1, switches1, rss = sample(daemon.pid)
        # A daemon that died while sampling keeps its counters until it is
        # reaped, so they would show a cost of nothing.
        if daemon.poll() is not None:
            sys.exit(f"the daemon exited while sampling ({daemon.returncode})")
    finally:
        status = terminate(daemon)
    hz = os.sysconf("SC_CLK_TCK")
    print(
        f"idle {seconds} s: {ticks1 - ticks0} CPU ticks ({(ticks1 - ticks0) / hz * 1000:.0f} ms), "
        f"{new_switches(switches0, switches1)} context switches, {len(switches1)} threads, RSS {rss / 1024:.1f} MiB; "
        f"exit status after SIGTERM: {status}"
    )
    if status != 0:
        sys.exit("the daemon did not exit cleanly after SIGTERM")


if __name__ == "__main__":
    main(sys.argv)
