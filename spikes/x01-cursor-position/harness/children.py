"""Helper windows the measurements start: a bounded wait for readiness."""

import os
import select
import subprocess
import time

READY_TIMEOUT = 15
MAX_LINE = 4096


def wait_ready(proc, timeout=READY_TIMEOUT):
    """The child's first line, read under one deadline. On any failure,
    including a signal, the child is stopped and reaped before re-raising."""
    try:
        line = read_line(proc.stdout.fileno(), time.monotonic() + timeout)
        if not line.startswith("ready"):
            raise RuntimeError(f"helper {proc.args!r} was not ready within {timeout} s: {line!r}")
        return line
    except BaseException:
        stop(proc)
        raise


def read_line(fd, deadline):
    """One line from fd, without ever blocking past the deadline. The fd's
    blocking mode is restored afterwards for the caller's own reader."""
    blocking = os.get_blocking(fd)
    os.set_blocking(fd, False)
    try:
        return _read_until(fd, deadline)
    finally:
        os.set_blocking(fd, blocking)


def _read_until(fd, deadline):
    data = b""
    while b"\n" not in data and len(data) < MAX_LINE:
        left = deadline - time.monotonic()
        if left <= 0 or not select.select([fd], [], [], left)[0]:
            break
        chunk = os.read(fd, MAX_LINE)
        if not chunk:
            break
        data += chunk
    return data.split(b"\n", 1)[0].decode(errors="replace").strip()


def stop(proc, timeout=5):
    """Closes the child's stdin (its signal to quit), then kills it if needed."""
    if proc.stdin and not proc.stdin.closed:
        proc.stdin.close()
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=timeout)
