#!/usr/bin/env python3
"""Upstream reproducer: KWin sends one PointerPositionChanged per earlier
QueryPointer call, not one in all, on the next pointer move.

Run under the spike lock: python3 harness/query_pointer_signals.py
"""

from bus import pump_for, query_pointer, session_bus
from inject import Injector


def main():
    bus = session_bus()
    signals = []
    bus.add_signal_receiver(
        lambda *_: signals.append(1),
        signal_name="PointerPositionChanged",
        dbus_interface="org.freedesktop.a11y.PointerLocator",
    )
    with Injector() as injector:
        injector.run("rel 0 0")
        pump_for(1.0)
        for calls in (1, 100):
            signals.clear()
            for _ in range(calls):
                query_pointer(bus)
            pump_for(0.2)
            injector.run("rel 5 0")
            pump_for(0.3)
            injector.run("rel -5 0")
            pump_for(0.5)
            print(f"{calls} QueryPointer calls, then 2 moves -> {len(signals)} PointerPositionChanged signals")


if __name__ == "__main__":
    main()
