"""Helper windows the measurements start: a bounded wait for readiness."""

import select
import subprocess

READY_TIMEOUT = 15


def wait_ready(proc, timeout=READY_TIMEOUT):
    """The child's first line, or kill and reap it if none comes in time."""
    readable, _, _ = select.select([proc.stdout], [], [], timeout)
    line = proc.stdout.readline() if readable else ""
    if isinstance(line, bytes):
        line = line.decode()
    if not line.startswith("ready"):
        stop(proc)
        raise RuntimeError(f"helper {proc.args!r} was not ready within {timeout} s")
    return line.strip()


def stop(proc, timeout=5):
    """Closes the child's stdin (its signal to quit), then kills it if needed."""
    if proc.stdin and not proc.stdin.closed:
        proc.stdin.close()
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=timeout)
