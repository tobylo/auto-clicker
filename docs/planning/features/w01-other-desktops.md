# W01: Run well on niri and Hyprland

- Issue: [#75](https://github.com/tobylo/auto-clicker/issues/75), a Feature after the 1.0 milestones (M0 to M4). Slices: W01a (#77), W01b (#80), W01c (#78), W01d (#81), W01e (#82). Follow-up outside this feature: W02 (#79), positions on niri.
- Maturity: refined 2026-11-12, ready to slice. Rests on [decision 0001](../../decisions/0001-cursor-position.md), [decision 0004](../../decisions/0004-pointer-mapping-on-niri-and-hyprland.md), the [R03 brief](r03-absolute-positions.md) and map D2, D3, D7, D11.
- Diagram: [start, detection, the layout and position sources, and where they are used](https://prlens.dev/c/9ovByKl0Mo9_uUmEkE4eaQ).

## Problem and outcome

Clicking, hotkeys, recording and playback of relative movement already work on niri and Hyprland on the kernel route (C02, C04 and R02 were measured on niri 26.04). Three things still exist only through KWin: the screen layout (so "Pick on the screen…" and the fixed point's screens check), and the pointer's position in recordings. With W01 the layout comes from xdg-output on every Wayland desktop, Hyprland recordings hold positions, and wherever a part cannot work the tray says why.

## Scope

In scope: a layout source on xdg-output for every desktop, Plasma included if it matches KWin's layout; picking the sources once at start; positions on Hyprland; the tray's and tooltip's wording for a missing source; the README for niri and Hyprland users.

Out of scope: GNOME; the ScreenCast portal as a position source; the portal route on niri and Hyprland (`auto` already falls back to the kernel route); a tray of the program's own (niri and Hyprland users run a bar with a StatusNotifierItem tray); positions on niri (W02, waiting for a niri release with ext-image-copy-capture cursor sessions, [0004](../../decisions/0004-pointer-mapping-on-niri-and-hyprland.md)); working around niri's rounding (decision 4 below).

## Invariants

These hold everywhere below and are stated once. R03's I1 to I5 still hold.

- **W1, sources are picked once, by what answers.** At start, after the single-instance name is claimed, the daemon picks a layout source and a position source by capability: KWin when `org.kde.KWin` has an owner (and, until W01b, its layout script), xdg-output when the Wayland connection offers `zxdg_output_manager_v1`, Hyprland's positions when its socket answers and the same Wayland connection is Hyprland's. Never by a desktop's name. A source that is missing leaves its part unknown for the daemon's life, with the reason kept for the tray; a restart tries again. backend-kwin keeps following `org.kde.KWin`'s owner as it does today.
- **W2, nothing outside a backend crate talks to a compositor.** The controller sees the layout and positions only through `Platform::layout` and `Platform::watch_layout`, whichever source detection picked.
- **W3, a layout starts at (0, 0).** niri and Hyprland allow outputs at negative positions and map the absolute device over the bounding box wherever it lies. The layout source moves the box to (0, 0) before `Layout::new`; a position from the compositor is moved by the offset of the layout it was taken on, never by a later one; the pick overlay moves winit's monitor corners by the same offset before matching them.
- **W4, a lost compositor is an unknown layout.** A Wayland connection that ends, or a protocol error on it, makes the layout unknown for the daemon's life, as losing KWin does today. It never ends the daemon. No reconnect: the session is ending.
- **W5, idle costs nothing.** The Wayland connection is read on tokio's `AsyncFd` and wakes only on the compositor's events. Positions are read only while a recording runs. Nothing is asked of a compositor on a timer.

## Behaviour

**Layout (W01a).** On a desktop with xdg-output and without KWin's layout script, the daemon binds every `wl_output` and its xdg-output, and takes each output's logical position and size, and its scale, once the output's `done` arrives. A layout is complete when every bound output has reported; an output added or removed, or any report that is not a layout (I1), makes it unknown until the next complete one. The watcher signals every change, as KWin's does. Picking the fixed point, the fixed point's screens check, and the screens check before positions are played then work as on Plasma. On niri at a fractional scale where physical/scale is not whole, a point near the far edges lands up to 1 logical px off (decision 4).

**Plasma (W01b).** W01a measures xdg-output's layout against KWin's layout script on the laptop: one and two outputs, scales 1, 1.25 and 1.5, with a fractional-width output left of and above the other. If they agree in every case, W01b moves Plasma's layout to xdg-output and removes the layout script; the position script stays, and positions are kept only while the layout its load reported equals the current one (R03's I4 rule, with xdg-output as the current layout). If they differ in any case, Plasma keeps its layout script, the comparison goes into decision 0004, and W01b closes with that evidence and no code.

**Hyprland positions (W01c, W01d).** W01c measures, on Hyprland 0.57 as the session on the laptop (two monitors) and the dev box: the absolute device's mapping on one and two outputs; xdg-output against Hyprland's own monitor sizes; and two position routes against R03's 2 ms delivery delay and 5 ms cover window. Route A asks `cursorpos` over Hyprland's socket on each evdev motion event, at most once per millisecond, with one last ask once the motion stops: the cost per ask to Hyprland's frame time, how late the answer is, and how often it describes the pointer before the move behind it. Route B uses ext-image-copy-capture cursor sessions: how late each position arrives, and whether moves arrive without a frame (VRR fullscreen). Route A is the default (decision 3). W01c brings the owner the choice with its numbers only if Route A misses the windows or costs a game frame time; the owner decides. W01d builds the route decided, feeding the recorder through `RecordingFeed` as KWin's watcher does, with the layout the position was taken on (W3). A floored `cursorpos` reply replays from the middle of its pixel.

**Saying what is missing (W01e).** When a recording keeps movement only because no position source exists on this desktop, the tray's Record item and tooltip say why, in the user's language (U04): no position source on this desktop, the compositor answered nothing, or no Wayland display. When the layout is unknown, picking says why as it does today, with the same reasons. The README recommends the flat pointer acceleration profile where positions are missing, and says how to start the user service on niri and on Hyprland: plain Hyprland never starts `graphical-session.target`, so the service needs uwsm or a line in Hyprland's config, and `WAYLAND_DISPLAY` must reach the systemd user environment.

## Decisions

Settled before refinement (the owner, 2026-11-12):
1. niri and Hyprland are the target desktops; GNOME is out of scope; the ScreenCast portal is out of scope unless the owner asks for a spike.
2. The kernel route already runs on niri; nothing in W01 changes clicking, hotkeys or relative recording.
3. Positions are per compositor: KWin's position script stays on Plasma. Where a compositor answers only when asked (Hyprland's `cursorpos`), the daemon asks on each evdev motion event with a cap, never on a timer.
4. Without positions, recordings keep relative movement and the tray says why; the README recommends the flat acceleration profile.
5. From R03: playback uses positions only on unchanged outputs, relative motion otherwise; a layout change stops a playback using positions.

Decided by the owner in refinement (2026-11-12):
6. **Five slices**, with a Hyprland measurement (W01c) before Hyprland positions are built, and the Plasma switch (W01b) apart from the new source.
7. **Wayland client:** `wayland-client` and `wayland-protocols`, both already in `Cargo.lock` through the settings window, pure Rust. A prototype measured 0 CPU ticks and 0 context switches over 60 s idle, +240 KiB resident and +202 KB of binary (decision 0004).
8. **niri's rounding:** accepted. niri maps the absolute device over ceil(physical/scale) while it reports round(physical/scale), so at such scales a point near the far edges lands up to 1 logical px off. Recorded in decision 0004 and the README; the owner gets the facts for an upstream report. No workaround: it would copy niri's rounding and break silently when niri changes it.
9. **Plasma:** moves to xdg-output in W01 if W01a's comparison agrees in every case (W01b); otherwise it keeps the layout script.
10. **Hyprland's route:** W01c measures both routes; decision 3's route stands unless the numbers show it costs a game frame time or misses R03's windows, and then the owner chooses.
11. **Hyprland is installed** on the laptop and the dev box for W01c; the owner logs into a Hyprland session once on each.

Defaults (reviewed; go ahead without asking):
12. Sources picked by capability, not name (W1); a stale `HYPRLAND_INSTANCE_SIGNATURE` whose socket does not answer, or that is not this Wayland display's compositor, gives no positions.
13. The box moves to (0, 0) (W3); no reconnect after the compositor goes (W4).
14. niri: no upstream request for a pointer command. Its maintainer has declined one (niri PR #1924) and points to ext-image-copy-capture cursor sessions, already on niri's main branch (0004). W02 takes it up once a release ships them.
15. Decision record 0004 holds every measurement, each marked measured or read from source; W01a and W01c add theirs.

## Slices

| Slice | Holds | Blocked by | Size |
| --- | --- | --- | --- |
| W01a: layout from xdg-output (#77) | New crate `backend-wayland`: the connection, the layout source and its watcher; detection at start (W1); the platforms hold the picked source; origin (W3) in the source and the pick overlay; the Plasma comparison recorded in 0004; manual check on niri | – | about 1,400 lines |
| W01b: Plasma on xdg-output (#80) | If W01a's comparison agrees: the layout from xdg-output on Plasma, the layout script removed, the position script's layout checked against it; Plasma checks. Otherwise closes with the evidence | W01a (#77) | about 800 lines |
| W01c: Hyprland measured (#78) | Investigation: Hyprland installed on both machines; mapping, xdg-output and both position routes measured; 0004 extended; the owner asked if decision 10 calls for it | – | evidence only |
| W01d: positions on Hyprland (#81) | New crate `backend-hyprland` (or Route B in `backend-wayland`): positions while recording, fed with their layout; Hyprland checks | W01a, W01c | about 1,200 lines |
| W01e: saying what is missing (#82) | Reasons from detection to the tray and tooltip (U04 wording, Swedish included); README for niri and Hyprland | W01a | about 500 lines |

## Acceptance criteria

1. On niri and Hyprland, the daemon starts, clicks, records and plays on the kernel route (manual check on each, versions in the PR). W01a for niri, W01d for Hyprland.
2. The layout comes from xdg-output on niri and Hyprland, follows screen changes, starts at (0, 0) whatever the outputs' positions, and maps a picked point with decision 0001's formula (W01a: tests against a fake source; manual check on niri; W01d: on Hyprland).
3. A lost Wayland connection or a protocol error makes the layout unknown and never ends the daemon (W01a, test).
4. Plasma: xdg-output's layout compared with KWin's in the cases above, recorded in 0004 (W01a); if they agree, Plasma's layout comes from xdg-output and positions still replay within 1 px (W01b, manual check on the laptop).
5. Each compositor's mapping of the absolute device across its screens is measured and recorded: niri in 0004 (one output; two outputs when a second monitor is at hand), Hyprland by W01c.
6. On Hyprland, recordings hold absolute positions and replay within 1 physical px per axis up to the scale W01c's numbers support (W01d, manual check, numbers in the PR).
7. Without a position source, recording keeps relative movement and the tray says why (W01e, test).
8. Idle cost unchanged: no polling, nothing asked of the compositor while idle beyond the layout's own events (W01a and W01d, `scripts/measure-daemon.py` in the PR).

## Open questions for the slices

W01a:
- The crate's shape: a source the platforms hold beside, or instead of, `Kwin`; how `Kernel` and `Portal` hold "KWin, xdg-output or none" without a second writer of the layout.
- How detection reaches a Wayland display the daemon was started without (no `WAYLAND_DISPLAY`): unknown, with that reason.
- Whether the xdg-output scale can be read exactly (it is not sent; `wl_output.scale` is whole) or is stored from the mode and logical size. Scale is stored, not compared (R03 decision 6).
- The spike injector's stale `Cargo.lock` (`spikes/x01-cursor-position/inject`): refresh it, or measure with the Python device as 0004 did.

W01b: whether the position script still reports the layout at its load, or positions are tied to the xdg-output layout current when each arrives.

W01d: the cap and the last ask (Route A), or the session per output and their enter and leave (Route B); pairing with clicks under R03's windows; the half-pixel replay of a floored position.

W01e: the exact wording of each reason, in English and Swedish.

## Needs the owner

- W01c: a login to a Hyprland session on the laptop and on the dev box.
- An upstream report to niri (or smithay) about the rounding, from the facts in 0004.
