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
    """Reads byte by byte up to the first newline, so nothing after it is
    taken from the caller's own reader; no newline in time is a failure."""
    data = b""
    while not data.endswith(b"\n"):
        if len(data) >= MAX_LINE or not readable_before(fd, deadline):
            raise RuntimeError(f"no complete line in time: {data!r}")
        byte = os.read(fd, 1)
        if not byte:
            raise RuntimeError(f"helper closed its output: {data!r}")
        data += byte
    return data.decode(errors="replace").strip()


def readable_before(fd, deadline):
    left = deadline - time.monotonic()
    return left > 0 and bool(select.select([fd], [], [], left)[0])


def stop(proc, timeout=5):
    """Closes the child's stdin (its signal to quit), then kills it if needed."""
    if proc.stdin and not proc.stdin.closed:
        proc.stdin.close()
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=timeout)
