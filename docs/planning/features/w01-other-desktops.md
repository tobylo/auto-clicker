# W01: Run well on niri and Hyprland

- Issue: [#75](https://github.com/tobylo/auto-clicker/issues/75), a Feature after the 1.0 milestones (M0 to M4). Slices: W01a (#77), W01b (#80), W01c (#78), W01d (#81), W01e (#82). Follow-up outside this feature: W02 (#79), positions on niri.
- Maturity: refined 2026-11-12, ready to slice. Rests on [decision 0001](../../decisions/0001-cursor-position.md), [decision 0004](../../decisions/0004-pointer-mapping-on-niri-and-hyprland.md), the [R03 brief](r03-absolute-positions.md) and map D2, D3, D4, D7, D11.
- Diagram: [start, detection, the layout and position sources, and where they are used](https://prlens.dev/c/9ovByKl0Mo9_uUmEkE4eaQ).

## Problem and outcome

Clicking, hotkeys, recording and playback of relative movement already work on niri and Hyprland on the kernel route (C02, C04 and R02 were measured on niri 26.04). Two things still come only from KWin: the screen layout, which "Pick on the screen…", the fixed point's screens check and playback's screens check need; and the pointer's position in recordings. With W01 the layout comes from xdg-output on every Wayland desktop, Hyprland recordings hold positions, and wherever a part cannot work the tray says why.

## Scope

In scope: a layout source on xdg-output for every desktop, Plasma included (it matches KWin's layout, decision 0004); picking the sources at start; positions on Hyprland; the tray's and tooltip's wording for a missing source; the README for niri and Hyprland users.

Out of scope: GNOME; the ScreenCast portal as a position source; the portal route on niri and Hyprland (`auto` already falls back to the kernel route); a tray of the program's own (niri and Hyprland users run a bar with a StatusNotifierItem tray); positions on niri (W02, waiting for a niri release with ext-image-copy-capture cursor sessions, [0004](../../decisions/0004-pointer-mapping-on-niri-and-hyprland.md)); working around niri's rounding (decision 8); a virtual device the user binds to one output in Hyprland's config (the README says positions and picking assume it is not).

## Invariants

These hold everywhere below and are stated once. R03's I1 to I5 still hold.

- **W1, sources are picked by what answers, and belong to the display.** KWin is followed through `org.kde.KWin`'s owner for the daemon's life, as today, whatever was found at start. The xdg-output source and Hyprland's position source are picked once at start, after the single-instance name is claimed, by capability: xdg-output when the Wayland connection offers `zxdg_output_manager_v1`, Hyprland's positions by the capability W01c's route needs. Never by a desktop's name. A compositor source counts only when it is the compositor of the daemon's Wayland display (KWin's owner, Hyprland's socket): another session's KWin on the same user bus, a nested compositor or a stale `HYPRLAND_INSTANCE_SIGNATURE` is ignored. A source that is missing leaves its part unknown, with the reason kept for the tray; a restart tries again. Until W01b, KWin's layout is used while KWin reports one, xdg-output's otherwise.
- **W2, nothing outside a backend crate talks to a compositor.** The controller sees the layout and positions only through `Platform::layout` and `Platform::watch_layout`.
- **W3, a layout is one coherent snapshot starting at (0, 0).** A layout is published only once every output's update of one change has arrived; until then it is unknown. niri and Hyprland allow outputs at negative positions and map the absolute device over the bounding box wherever it lies, so the source moves the box to (0, 0) before `Layout::new`, and the pick overlay moves winit's monitor corners by the same offset.
- **W4, a position belongs to the layout it was taken on.** A position is moved by that layout's offset and checked against it, never against a later one. When the position and the layout arrive on different channels (a socket or D-Bus beside the Wayland connection) and the position carries no mark of its layout, a position stamped within a bound before a layout change is dropped, so one taken on the new screens is never written against the old. The bound is how late that compositor's layout notice can trail a position on the new screens, measured with margin (W01c for Hyprland, W01b for Plasma); a source whose positions carry their layout (a cursor session on the Wayland connection, a script's report) uses that instead.
- **W5, a lost compositor.** A Wayland connection that ends, or a protocol error on it, makes the layout unknown and never ends the daemon. On niri and Hyprland it stays unknown for the daemon's life: their session is ending. On Plasma, KWin restarts within the session (`kwin_wayland_wrapper`), so a new owner of `org.kde.KWin` connects again, as the scripts load again today.
- **W6, every request to a compositor is bounded.** R03's I5 applies to every compositor, not only KWin: each request has its own deadline, at most one is in flight, its reply is read to a small fixed limit and parsed once into a validated type, and a failure or a late reply is no position, never a failed or blocked recording. Every request, deadline and session a recording starts ends with it.
- **W7, idle costs nothing; locked motion stays relative.** The Wayland connection wakes only on the compositor's events. Positions are read only while a recording runs: asked on motion, at its start, and once more at most 1 ms after the last motion the per-millisecond cap held back (a deadline bound to the recording, not a poll). While a game holds the pointer, no position may cover its relative moves (map D4): a route that cannot tell is not used.

## Behaviour

**Layout (W01a).** On a desktop with xdg-output, the daemon binds every `wl_output` (up to `MAX_OUTPUTS`) and its xdg-output, and takes each output's logical position and size. The watcher signals every change, as KWin's does. Picking the fixed point, the fixed point's screens check and playback's screens check then work as on Plasma. On niri at a fractional scale where physical/scale is not whole, a point near the far edges lands up to 1 logical px off (decision 8).

**Plasma (W01b).** xdg-output's layout matched KWin's on the laptop in all seven layouts tried, at scales 1, 1.25 and 1.5, with fractional-width outputs left of and above the other (decision 0004, 2026-11-12). So W01b moves Plasma's layout to xdg-output and removes the layout script. The position script stays and still reports the layout at its load; its positions are kept only while that layout equals xdg-output's current one, under W4. A KWin restart reconnects (W5).

**Hyprland positions (W01c, W01d).** W01c measures, on Hyprland 0.57 as the session on the laptop (two monitors) and the dev box: the absolute device's mapping on one and two outputs, a negative position included; xdg-output against Hyprland's monitor sizes; and two position routes. Route A asks `cursorpos` over Hyprland's socket on each evdev motion event, at most once per millisecond. Route B uses ext-image-copy-capture cursor sessions. For each: the cost to Hyprland's frame time while a game runs (Route A opens a connection per ask), how late a position is against R03's 2 ms delivery delay and 5 ms cover window, how often it describes the pointer before the move behind it, whether it keeps a locked pointer's moves uncovered (W7), and how late xdg-output's notice of a layout change trails a position on the new screens (W4's bound). Route A is the default (decision 3). W01c brings the owner the choice with its numbers if Route A misses the windows, costs a game frame time or fails W7; the owner decides. W01d builds the route decided, feeding the recorder through `RecordingFeed` as KWin's watcher does, with the layout each position was taken on (W4).

**Saying what is missing (W01e).** When a recording keeps movement only because no position source exists on this desktop, or no Wayland display was given, the tray's Record item and tooltip say why, in the user's language (U04), and picking says why its screens are unknown. W01d adds Hyprland's own reasons (its socket did not answer, or belongs to another display) with their wording. The README recommends the flat pointer acceleration profile where positions are missing; says how to start the user service on niri and on Hyprland (plain Hyprland never starts `graphical-session.target`, so uwsm or a line in its config; `WAYLAND_DISPLAY` and, on Hyprland, `HYPRLAND_INSTANCE_SIGNATURE` in the systemd user environment, cleared when the session ends); and brings its statements about picking and positions in step with W01 (niri's rounding, picking without KWin, positions on Hyprland).

## Decisions

Settled before refinement (the owner, 2026-11-12):
1. niri and Hyprland are the target desktops; GNOME is out of scope; the ScreenCast portal is out of scope unless the owner asks for a spike.
2. The kernel route already runs on niri; nothing in W01 changes clicking, hotkeys or relative recording.
3. Positions are per compositor: KWin's position script stays on Plasma. Where a compositor answers only when asked (Hyprland's `cursorpos`), the daemon asks on each evdev motion event with a cap, never on a timer. W7's last ask is a deadline set by motion while recording, not a poll.
4. Without positions, recordings keep relative movement and the tray says why; the README recommends the flat acceleration profile.
5. From R03: playback uses positions only on unchanged outputs, relative motion otherwise; a layout change stops a playback using positions.

Decided by the owner in refinement (2026-11-12):
6. **Five slices**, with a Hyprland measurement (W01c) before Hyprland positions are built, and the Plasma switch (W01b) apart from the new source.
7. **Wayland client:** `wayland-client` and `wayland-protocols`, both already in `Cargo.lock` through the settings window, pure Rust. A prototype measured 0 CPU ticks and 0 context switches over 60 s idle, +240 KiB resident and +202 KB of binary (decision 0004).
8. **niri's rounding:** accepted. niri maps the absolute device over ceil(physical/scale) while xdg-output reports round(physical/scale), so where they differ a point near the far edges lands up to 1 logical px off. Recorded in decision 0004 and the README; the owner gets the facts for an upstream report. No workaround: it would copy niri's rounding and break silently when niri changes it.
9. **Plasma:** moves to xdg-output in W01 if xdg-output agrees with KWin's layout in every case measured. It did (decision 0004), so W01b makes the move.
10. **Hyprland's route:** W01c measures both routes; decision 3's route stands unless the numbers show it costs a game frame time, misses R03's windows or fails W7, and then the owner chooses.
11. **Hyprland is installed** on the laptop and the dev box for W01c; the owner logs into a Hyprland session once on each.

Defaults (reviewed; go ahead without asking):
12. Invariants W1 to W7.
13. niri: no upstream request for a pointer command. Its maintainer has declined one (niri PR #1924) and points to ext-image-copy-capture cursor sessions, already on niri's main branch (0004). W02 takes it up once a release ships them.
14. Decision record 0004 holds every measurement, each marked measured or read from source; W01a and W01c add theirs.

## Slices

| Slice | Holds | Blocked by | Size |
| --- | --- | --- | --- |
| W01a: layout from xdg-output (#77) | New crate `backend-wayland`: the connection, the layout source and its watcher (W3, W5); detection of the xdg-output source and the display check for KWin (W1); the platforms hold the picked source; the pick overlay's offset; niri's two-output mapping recorded in 0004; manual check on niri | – | about 1,400 lines |
| W01b: Plasma on xdg-output (#80) | The layout from xdg-output on Plasma, the layout script removed, the position script's layout checked against it (W4), reconnecting after a KWin restart (W5); Plasma checks | W01a | about 800 lines |
| W01c: Hyprland measured (#78) | Investigation: Hyprland installed on both machines; mapping, xdg-output and both routes measured; 0004 extended; the owner asked if decision 10 calls for it | – | evidence only |
| W01d: positions on Hyprland (#81) | Detection of the route's capability and its display check (W1); positions while recording, bounded (W6), with their layout (W4), at the start and after the last motion (W7); Hyprland's reasons and their wording; Hyprland checks | W01a, W01c | about 1,200 lines |
| W01e: saying what is missing (#82) | The reasons W01a's detection gives, to the tray, tooltip and picking (U04 wording, Swedish included); the README | W01a | about 500 lines |

## Acceptance criteria

1. On niri and Hyprland, the daemon starts, clicks, records and plays on the kernel route (manual check on each, versions in the PR): W01a for niri, W01d for Hyprland.
2. The layout comes from xdg-output on niri and Hyprland, is published only whole, follows screen changes, starts at (0, 0) whatever the outputs' positions, and maps a picked point with decision 0001's formula (W01a: tests against a fake source, an update to two outputs interleaved included; manual check on niri; W01d: on Hyprland).
3. A lost Wayland connection or a protocol error makes the layout unknown and never ends the daemon; a source of another display is ignored (W01a, tests).
4. Plasma: xdg-output's layout matches KWin's (decision 0004, measured in refinement). Plasma's layout comes from xdg-output, positions still replay within D3's 0.65 physical px per axis at scales 1 to 1.5, and a KWin restart brings the layout back (W01b, manual checks on the laptop).
5. Each compositor's mapping of the absolute device across its screens is measured and recorded: niri on one output in 0004, on two outputs (one at a negative position) by W01a; Hyprland by W01c.
6. On Hyprland, recordings hold absolute positions and replay within 1 physical px per axis up to the scale W01c's numbers support; a click made without moving replays on its pixel; a recording from the desktop into a pointer-locked game and back replays both parts (W01d, manual checks, numbers in the PR).
7. Every request to Hyprland is bounded: one that never answers costs the recording its positions and nothing else; a position taken before a layout change is not written against the new layout (W01d, tests against a fake source).
8. Without a position source, recording keeps relative movement and the tray says why (W01e, test; W01d for Hyprland's reasons).
9. Idle cost unchanged: no polling, nothing asked of a compositor while idle beyond the layout's own events (W01a and W01d, `scripts/measure-daemon.py` in the PR).

## Open questions for the slices

W01a:
- How `Kernel` and `Portal` hold KWin, xdg-output or none without a second writer of the layout.
- How a whole layout update is recognised (a round trip after the first `done`, say), for xdg-output v3, which sends `done` on `wl_output`, and older versions, which send it on their own object.
- How KWin's owner is matched to the Wayland display (W1), without a check by name.
- The scale stored per output: xdg-output sends none and `wl_output.scale` is whole; derived from the mode, the transform taken into account, and the logical size. Scale is stored, not compared.
- The offset's arithmetic (`checked_*` on negative positions); a daemon started without `WAYLAND_DISPLAY`.
- The X01 spike injector's stale `Cargo.lock` (`spikes/x01-cursor-position/inject`): refresh it, or measure with the Python device as 0004 did.
- niri on two outputs needs a second monitor on the dev box, or niri on the laptop (Needs the owner).

W01b: how the position script's layout and xdg-output's are compared under W4 when they arrive on D-Bus and Wayland, and how late KWin's xdg-output notice trails its position script after a layout change (W4's bound).

W01d:
- Route A: the socket path built only from a `HYPRLAND_INSTANCE_SIGNATURE` that is one path component; the reply's exact grammar (signed whole numbers); the cap, the first ask and the last ask. Route B: one session per output, their enter and leave, and Hyprland's permission prompt when `enforce_permissions` is on.
- Replaying a floored `cursorpos` from the middle of its pixel needs the recording to say how its positions were rounded: a format version 3 (R03 decision 7), or accepting floor's error within criterion 6's scale.

W01e: the exact wording of each reason, in English and Swedish.

## Needs the owner

- W01c: a login to a Hyprland session on the laptop and on the dev box.
- W01a: a second monitor on the dev box, or niri installed on the laptop, for niri's two-output mapping.
- An upstream report to niri (or smithay) about the rounding, from the facts in 0004.
