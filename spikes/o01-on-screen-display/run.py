#!/usr/bin/env python3
"""Drives one O01a run: a test window, the display above it, and virtual
devices that inject only into that window.

    run.py WINDOW SCENARIO [OUTPUT]

WINDOW is probe-, sdl-, x11- or xlib- with locked or free: the probe's own
Wayland window, sdlwin.py with SDL's wayland or x11 driver, or xwin.py, an
X11 window that grabs and warps the pointer as older and Wine games do. SCENARIO is control (no display), number (the countdown appears
under the pointer, changes twice and goes while input flows), badge, or both.
OUTPUT names the screen (the first one by default).

Locked windows get relative moves at 1 kHz, a right Shift tap every 250 ms
and a left click every 500 ms for 3 s; free windows get clicks at the
picture's place. Nothing more is injected once the window's latest lines
say it lost focus or its lock, and every device is released and destroyed
on the way out. Prints a summary line per case: sent against received, and
VOID instead of PASS or FAIL when the display never drew, ended with an
error or died, the guard stopped injection early, or the window never got
focus, its lock or relative mode (then nothing is injected at all).
"""

import os
import queue
import subprocess
import sys
import threading
import time
from dataclasses import dataclass

from uinput import Devices

HERE = os.path.dirname(os.path.abspath(__file__))
PROBE = os.path.join(HERE, "target/release/o01-probe")
SDLWIN = os.path.join(HERE, "sdlwin.py")
BAD = ("LOST", "UNLOCKED", "ended:")
BADGE_CENTRE = 24 + 20
PRE_SHOWN = {}
# ONE_WAY=1 sends every move as (+1, 0), so that merged moves keep their sum
# while dropped ones do not.
STEP = (lambda tick: (1, 0)) if os.environ.get("ONE_WAY") else (lambda tick: (3 if tick % 2 == 0 else -3, 0))


class Process:
    """A child whose lines are printed with a tag and kept."""

    def __init__(self, tag, argv, env=None):
        self.tag = tag
        self.lines = queue.Queue()
        self.seen = []
        self.proc = subprocess.Popen(["setpriv", "--pdeathsig", "TERM", "--", *argv], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, text=True, bufsize=1, env=env)
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        for line in self.proc.stdout:
            line = line.rstrip("\n")
            if not line.split(" ", 1)[-1].startswith(("relative total", "absolute total")):
                print(f"[{self.tag}] {line}", flush=True)
            self.seen.append(line)
            self.lines.put(line)

    def alive(self):
        """Whether the child is still running; one that died says so here even
        when it printed nothing."""
        return self.proc.poll() is None

    def send(self, command):
        try:
            self.proc.stdin.write(command + "\n")
            self.proc.stdin.flush()
        except BrokenPipeError:
            pass

    def wait_for(self, word, seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            try:
                if word in self.lines.get(timeout=0.05):
                    return True
            except queue.Empty:
                pass
        return False

    def stop(self):
        self.send("quit")
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=3)
        except (subprocess.TimeoutExpired, BrokenPipeError):
            self.proc.kill()
            self.proc.wait()


def outputs():
    """Each output's name and logical box, from the probe."""
    text = subprocess.run([PROBE, "outputs"], capture_output=True, text=True, check=True).stdout
    found = {}
    for line in text.splitlines():
        words = line.split()
        if "is" in words and "at" in words:
            width, height = map(int, words[words.index("is") + 1].split("x"))
            x, y = map(int, words[words.index("at") + 1].split(","))
            found[words[2]] = (x, y, width, height)
    return found


def bounding(boxes):
    left = min(x for x, _, _, _ in boxes)
    top = min(y for _, y, _, _ in boxes)
    right = max(x + w for x, _, w, _ in boxes)
    bottom = max(y + h for _, y, _, h in boxes)
    return left, top, right - left, bottom - top


def window_argv(kind, output):
    source, mode = kind.split("-")
    if source == "probe":
        return [PROBE, "window", mode, output], None
    if source == "xlib":
        return [sys.executable, os.path.join(HERE, "xwin.py"), mode], None
    env = dict(os.environ, SDL_VIDEO_DRIVER="x11" if source == "x11" else "wayland")
    return [sys.executable, SDLWIN, mode], env


class Tally:
    """What the window said so far: relative moves, keys, buttons, and
    lines that say something went wrong."""

    def __init__(self, window):
        self.window = window

    def now(self):
        lines = list(self.window.seen)
        texts = [line.split(" ", 1)[-1] for line in lines]
        totals = [int(text.split()[2]) for text in texts if text.startswith("relative total")]
        keys = sum(map(is_key_press, texts))
        presses = sum(map(is_button_press, texts))
        bad = [line for line, text in zip(lines, texts) if is_bad(text)]
        return (totals[-1] if totals else 0), keys, presses, bad


def is_key_press(text):
    return text.startswith("key ") and "Pressed" in text


def is_button_press(text):
    return text.startswith("button ") and "press" in text


def is_bad(text):
    return any(word in text for word in BAD)


SCRIPTS = {
    "control": [],
    "number": [(0.3, "start"), (0.3, "number 3"), (1.0, "number 2"), (1.5, "number 1"), (2.0, "none")],
    "badge": [(0.3, "start"), (0.3, "badge"), (2.0, "none")],
    "both": [(0.3, "start"), (0.3, "both 3"), (1.0, "both 2"), (2.0, "none")],
    # One transition each, the number already shown where PRE says so.
    "appear": [(0.3, "start"), (0.3, "number 3")],
    "swap": [(1.0, "number 2"), (1.5, "number 1"), (2.0, "number 3")],
    "remove": [(1.5, "none")],
    "badge-remove": [(1.5, "none")],
}

# Shown before the run's first tally, so only the script's transition counts.
PRE = {"swap": "number 3", "remove": "number 3", "badge-remove": "badge"}


class NotStarted(Exception):
    """The window never got into the state the run needs; nothing was injected."""


@dataclass
class Run:
    """One run: the window kind, the scenario, where to aim, and what was made."""

    kind: str
    scenario: str
    name: str
    target: tuple
    box: tuple
    devices: object = None
    window: object = None
    # Set when the guard stopped injection: the run measured less than it says.
    aborted: bool = False

    @property
    def locked(self):
        return self.kind.endswith("locked")


def show(command, settle):
    """A display showing `command`'s picture, once it drew it and `settle` s more."""
    display = Process("display", [PROBE, "show"])
    display.send(command)
    display.wait_for("drew", 3)
    time.sleep(settle)
    return display


def follow(script, elapsed, display):
    """Sends the display the script's commands that are due; "start" starts it."""
    while script and script[0][0] <= elapsed:
        _, command = script.pop(0)
        if command == "start":
            display = Process("display", [PROBE, "show"])
        else:
            display.send(command)
    return display


def last_state(window, prefix):
    """The rest of the window's latest line starting with `prefix`, or None."""
    for line in reversed(list(window.seen)):
        text = line.split(" ", 1)[-1]
        if text.startswith(prefix):
            return text[len(prefix):]
    return None


def holds(window):
    """Whether the window has keyboard focus and, if it reported a lock, the
    lock, by its latest lines (a flicker at startup that ends in "gained"
    counts as held), and has not ended."""
    ended = not window.alive() or any("ended:" in line for line in list(window.seen))
    focused = last_state(window, "keyboard focus: ") == "gained" and last_state(window, "pointer focus: ") != "LOST"
    return not ended and focused and last_state(window, "pointer lock: ") != "UNLOCKED"


def inject(devices, tick, sent):
    """One millisecond's input: a move, a Shift tap every 250 ms, a click every 500 ms."""
    devices.move(*STEP(tick))
    sent[0] += 1
    if tick % 250 == 0:
        devices.key(True)
        devices.key(False)
        sent[1] += 1
    if tick % 500 == 250:
        devices.button(True)
        devices.button(False)
        sent[2] += 1


def run_locked(run, length=3.0):
    """Injects for `length` s while the display follows its script; returns
    what was sent and the display process, if one was started."""
    script = list(SCRIPTS[run.scenario])
    display, sent = PRE_SHOWN.get("display"), [0, 0, 0]
    start = time.monotonic()
    tick = 0
    while (elapsed := time.monotonic() - start) < length:
        display = follow(script, elapsed, display)
        if not holds(run.window):
            print("--- the window lost focus or its lock: injection stopped", flush=True)
            run.aborted = True
            break
        inject(run.devices, tick, sent)
        tick += 1
        time.sleep(max(0.0, start + tick / 1000 - time.monotonic()))
    return sent, display


def click_three(run, sent):
    """Three clicks, each only while the window still holds focus."""
    for _ in range(3):
        if not holds(run.window):
            print("--- the window lost focus: no more clicks", flush=True)
            run.aborted = True
            return
        devices = run.devices
        devices.button(True)
        time.sleep(0.03)
        devices.button(False)
        sent[2] += 1
        time.sleep(0.2)


def run_free(run):
    """Clicks three times at the picture's place with it shown, then three
    times after it went."""
    sent = [0, 0, 0]
    run.devices.place(*run.target, run.box)
    time.sleep(0.3)
    display = None if run.scenario == "control" else show(SCRIPTS[run.scenario][1][1], 0.3)
    click_three(run, sent)
    if display:
        display.send("none")
        time.sleep(0.4)
    click_three(run, sent)
    return sent, display


def plan():
    """The run the command line asks for, aimed at the chosen output."""
    kind, scenario = sys.argv[1], sys.argv[2]
    if kind.endswith("free") and scenario not in ("control", "number", "badge", "both"):
        sys.exit(f"a free window takes control, number, badge or both, not {scenario}")
    layout = outputs()
    name = sys.argv[3] if len(sys.argv) > 3 else next(iter(layout))
    ox, oy, ow, oh = layout[name]
    box = bounding(layout.values())
    centre = (ox + ow / 2, oy + oh / 2)
    target = (ox + BADGE_CENTRE, oy + BADGE_CENTRE) if scenario.startswith("badge") else centre
    print(f"--- {kind} {scenario} on {name} {layout[name]}, box {box}, target {target}", flush=True)
    return Run(kind, scenario, name, target, box)


def fullscreen_by_command():
    if os.environ.get("FULLSCREEN_CMD"):
        # For a window the compositor does not make fullscreen on its own
        # request (an X11 window under xwayland-satellite).
        subprocess.run(os.environ["FULLSCREEN_CMD"].split(), check=False)
        time.sleep(1.0)


def await_lock(run):
    run.devices.move(1, 0)
    if run.kind.startswith("probe") and not run.window.wait_for("pointer lock: locked", 5):
        raise NotStarted("the window never locked the pointer; nothing injected")
    if run.kind.startswith("xlib") and not said(run.window, "grabbed=True"):
        raise NotStarted("the X11 window never grabbed the pointer; nothing injected")
    time.sleep(0.5)


def said(window, text):
    """Whether the window printed `text` in any line so far."""
    return any(text in line for line in list(window.seen))


def fullscreen(window):
    """Whether the probe's window was last configured fullscreen."""
    configured = [line for line in list(window.seen) if "window configured" in line]
    return bool(configured) and "fullscreen=true" in configured[-1]


def check_started(run):
    """Refuses a probe window that is not fullscreen, and an SDL window
    refused relative mode."""
    if run.kind.startswith("probe") and not fullscreen(run.window):
        raise NotStarted("the window is not fullscreen; nothing injected")
    if said(run.window, "relative mode: False"):
        raise NotStarted("the window was refused relative mode; nothing injected")


def start_window(run):
    """Starts the window under the pointer and waits for its focus, and its lock if locked."""
    time.sleep(1.0)
    run.devices.place(*run.target, run.box)
    argv, env = window_argv(run.kind, run.name)
    run.window = Process("window", argv, env)
    if not run.window.wait_for("keyboard focus: gained", 8):
        raise NotStarted("the window never got keyboard focus; nothing injected")
    fullscreen_by_command()
    time.sleep(0.5)
    run.devices.place(*run.target, run.box)
    check_started(run)
    if run.locked:
        await_lock(run)


def write_trace(run, display):
    with open(os.environ["TRACE"], "a") as trace:
        trace.write(f"--- {run.kind} {run.scenario} {run.name}\n" + "\n".join(run.window.seen) + "\n")
        trace.write("\n".join("display " + line for line in (display.seen if display else [])) + "\n")


def display_worked(display, screen):
    """Whether the display drew on `screen`, kept its surfaces there, is
    still running and printed no error."""
    if not display or not display.alive():
        return False
    drew = said(display, f" on {screen}: buffer")
    closed = said(display, f" on {screen} (closed by the compositor)")
    failed = said(display, " ended: ") or said(display, " no buffer")
    return drew and not (closed or failed)


def shown(run, display):
    """Whether the run measured what it claims: injection was not stopped
    early, an SDL window got relative mode, and (unless it is a control) the
    display worked."""
    refused = any("relative mode: False" in line for line in list(run.window.seen))
    if run.aborted or refused:
        return False
    return run.scenario == "control" or display_worked(display, run.name)


def judge(run, before, sent, display):
    """Prints the verdict: what was sent against what the window got since `before`."""
    after = Tally(run.window).now()
    got = [after[i] - before[i] for i in range(3)]
    bad = after[3][len(before[3]):]
    verdict = "PASS" if got == sent and not bad else "FAIL"
    if not shown(run, display):
        verdict = "VOID"
    sums = [line.split()[-1] for line in run.window.seen if " relative total " in line][-1:]
    if os.environ.get("TRACE"):
        write_trace(run, display)
    print(f"=== {verdict} {run.kind} {run.scenario} on {run.name}: sent moves/keys/clicks {sent}, got {got}, "
          f"problems {bad}, last relative sum {sums}", flush=True)


def main():
    run = plan()
    run.devices = Devices()
    display = None
    try:
        start_window(run)
        if run.scenario in PRE:
            PRE_SHOWN["display"] = show(PRE[run.scenario], 0.5)
        before = Tally(run.window).now()
        sent, display = run_locked(run) if run.locked else run_free(run)
        time.sleep(0.6)
        judge(run, before, sent, display)
    except NotStarted as reason:
        if os.environ.get("TRACE"):
            write_trace(run, display)
        print(f"=== VOID {run.kind} {run.scenario} on {run.name}: {reason}", flush=True)
    finally:
        try:
            run.devices.close()
        finally:
            for child in (display, run.window):
                if child:
                    child.stop()


if __name__ == "__main__":
    main()
