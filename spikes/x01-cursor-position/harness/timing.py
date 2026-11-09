"""Latency and CPU of the KWin-script route.

latency: absolute moves to distinct positions, paced at a rate; each Position
call is matched to the move that caused it, through the mapping formula.
cpu: relative moves at a rate (or none, for idle) with the script on, off,
or loaded with no receiver; CPU of KWin, the bus and the receiver.
"""

import os
import time

from bus import Receiver, loaded_script, pump_for, pump_until, session_bus
from inject import Injector, Session, to_absolute, wait_for_pointer
from stats import TICK, cpu_ticks, pids, summary

STEP = 64  # absolute units per move: 2 logical px at a 2048 px wide layout


def current_layout(receiver):
    if not pump_until(lambda: receiver.layouts, 5):
        raise RuntimeError("the script never reported the layout")
    return receiver.layouts[-1]


def latency(hz, sweeps, count):
    bus = session_bus()
    receiver = Receiver(bus)
    with loaded_script(bus), Injector() as injector:
        layout = current_layout(receiver)
        wait_for_pointer(injector, receiver, layout)
        session = Session(injector, receiver, layout)
        rows = [sweep(session, hz, count) for _ in range(sweeps)]
    report_latency(hz, rows)


def sweep(session, hz, count):
    """One run of `count` moves left to right along the middle row."""
    injector, receiver, layout = session.injector, session.receiver, session.layout
    y = to_absolute(layout.height // 2, layout.height)
    injector.run(f"abs 0 {y}")
    pump_for(0.2)
    first = len(receiver.samples)
    sent = injector.run(f"stream-abs {hz} {count} {STEP} {y} {STEP}")
    pump_for(0.3)
    samples = receiver.samples[first:]
    matched = match(samples, sent, layout.width)
    return {"sent": len(sent), "samples": samples, "matched": matched}


def match(samples, sent, width):
    """(sample, move index) for every sample that maps back onto a move."""
    pairs = []
    for sample in samples:
        index = round(sample.x * 65536 / width / STEP) - 1
        if 0 <= index < len(sent):
            pairs.append((sample, sent[index]))
    return pairs


def report_latency(hz, rows):
    sent = sum(row["sent"] for row in rows)
    received = sum(len(row["samples"]) for row in rows)
    callbacks = sum(row["samples"][-1].callbacks - row["samples"][0].callbacks + 1 for row in rows if row["samples"])
    end_to_end = [(s.recv_ns - t) / 1e6 for row in rows for s, t in row["matched"]]
    script_to_receipt = [s.recv_wall_ms - s.script_ms for row in rows for s in row["samples"]]
    print(f"rate {hz} Hz: moves {sent}, callbacks {callbacks}, received {received}")
    print(f"  uinput write -> receipt: {summary(end_to_end, 'ms')}")
    print(f"  script callback -> receipt (ms clock): {summary(script_to_receipt, 'ms')}")


def cpu(hz, seconds, mode):
    """mode: off (no script), on (script and receiver), dead (script, no receiver)."""
    bus = session_bus()
    receiver = Receiver(bus) if mode == "on" else None
    if mode == "off":
        return measure_cpu(hz, seconds, mode, receiver)
    with loaded_script(bus):
        return measure_cpu(hz, seconds, mode, receiver)


def measure_cpu(hz, seconds, mode, receiver):
    targets = {"kwin_wayland": pids("kwin_wayland"), "dbus-broker": pids("dbus-broker")}
    with Injector() as injector:
        pump_for(1.0)
        before = snapshot(targets)
        start = time.monotonic()
        if hz:
            injector.run(f"stream-rel {hz} {hz * seconds} 2", timeout=seconds + 30)
        else:
            pump_for(seconds)
        elapsed = time.monotonic() - start
        after = snapshot(targets)
    received = len(receiver.samples) if receiver else 0
    print(f"cpu rate {hz} Hz, script {mode}, {elapsed:.1f} s, received {received}: " + usage(elapsed, before, after))


def snapshot(targets):
    ticks = {name: sum(cpu_ticks(pid) for pid in found) for name, found in targets.items()}
    own = os.times()
    ticks["receiver"] = (own.user + own.system) * TICK
    return ticks


def usage(elapsed, before, after):
    parts = [f"{name} {(after[name] - before[name]) / TICK / elapsed * 100:.2f}%" for name in before]
    return ", ".join(parts)
