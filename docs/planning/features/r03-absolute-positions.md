# R03: Absolute cursor positions in recordings

- Issue: [#16](https://github.com/tobylo/auto-clicker/issues/16), a Feature under [G2 (#3)](https://github.com/tobylo/auto-clicker/issues/3), milestone M2. Slices: R03a (#51), R03b (#52).
- Maturity: refined 2026-11-10, ready to slice. Rests on [decision 0001](../../decisions/0001-cursor-position.md) (X01) and map D3, D4, D7, D11. Where this brief and decision 0001 differ (two scripts, the layout rule), this brief holds; the decision record marks those places.
- Diagram: [the flow from KWin to the file and back](https://prlens.dev/c/SSdbz2qLBRoW1UnAP_2jtw).

## Problem and outcome

A recording stores relative motion only. Replayed on the desktop it passes through pointer acceleration, so 400 units land 397 to 795 px away depending on speed (R02). With R03, a recording made on the desktop replays its clicks to the pixels they were made at, within 1 px per axis at scales up to about 1.8 (decision 0001). In a game that locks the pointer nothing changes: relative motion is recorded and replayed.

## Scope

In scope: reading the pointer position and the output layout from KWin while recording; storing both in the recording; replaying positions as absolute moves; falling back to relative motion; loading, unloading and cleaning up the KWin scripts.

Out of scope: sub-pixel positions (upstream wish 3 in decision 0001); rescaling positions to another layout; positions on compositors other than KWin; recording through the portal (P03); replacing the Ctrl+F7 hotkey (map, open questions).

## Invariants

These hold everywhere below and are stated once.

- **I1, a layout.** A layout is one or more outputs, each a rectangle of positive area in whole logical px with a scale, in a bounded number, whose bounding box starts at (0, 0) (KWin's own layouts always do, decision 0001). Anything else, from a file or from D-Bus, is not a layout: a file holding it is refused, and a report of it leaves the current layout unknown.
- **I2, a position.** A recorded position lies on at least one output of its recording's layout. The recorder drops a report that does not; a file holding one is refused.
- **I3, one validated form.** Reports from KWin are parsed once into the same types the file is parsed into. No sequence of reports makes a recording fail to finish, and the recorder never writes a file the program then refuses to load, by its content or its size.
- **I4, the current layout is known or unknown.** It is unknown until the layout script loaded by this daemon has reported with the token of that load, and becomes unknown again the moment `org.kde.KWin` changes or loses its owner, or a load fails.
- **I5, every call to KWin is bounded** by its own deadline and ends at once on shutdown. A failure never blocks or fails a recording or the start of a playback: the recording carries on without positions, and a playback that starts while the layout is unknown uses relative motion. A playback already using positions is stopped instead ("Playback").

## Behaviour

**Recording.**
- While a recording runs, a KWin script reports `workspace.cursorPos` (whole logical px) once at load and on every change that differs from the last one sent. The daemon stamps each report on the event clock when it arrives.
- The recorder writes positions as events of their own, beside the relative moves it already writes. It keeps at most one position per millisecond, the latest; a press, a release or a turn of the wheel ends that millisecond, so the position before a click or a scroll is never replaced by one after it. The first position after such an event is kept as it is, and counting starts with the next one: it may belong to the move before the event, and a later one must not stand in for it.
- The first position of a recording is written at the recording's start when no relative move came before it: the pointer has not moved since, so input in the few milliseconds the script takes to load still has its position.
- Events are ordered by time, not by arrival. A report is never stamped earlier than the move that caused it, so an input event read late goes before the positions stamped after it, where today it would be moved forward.
- A recording has one layout: the current one at its start. Positions are written only while the current layout is known and equal to it. So a recording started while the layout is unknown has no positions; a layout change ends them for good or until the layout is the recording's again; and after a KWin restart they resume only once the new layout script has reported the same layout.
- An idle recording still saves nothing: positions alone do not make a recording. The trip to the tray is trimmed as today; positions inside the trip go with its motion.

**Playback.**
- Positions are used when the current layout is known and every output a recorded position lies on is present in it with the same rectangle. A monitor added to the right of or below unchanged screens keeps positions exact; a screen that moved, changed size or is gone does not. Scale is not compared.
- Otherwise the whole run uses relative motion, as R02 does. When the recording holds positions that cannot be used, the tray tooltip says why while it plays; a recording without positions plays with nothing new shown.
- When positions are used, each is sent as an absolute move to `clamp(round(x * 65536 / W), 0, 65535)` (likewise y with H), W and H being the size of the current layout's bounding box (decision 0001's formula).
- A relative move is covered when a position follows it within 5 ms in the recording. A covered move is not sent. Every other relative move is sent (a locked pointer, KWin restarting, the pointer pushed against an edge).
- A press, a release or a turn of the wheel is never played between a covered move and the position covering it: that position is sent first, at that event's time, provided it follows the event within the delivery delay. A position that comes later than that belongs to motion after the event and stays where it is, and the move before the event is then sent as relative motion. A scroll acts on the window under the pointer, as a click does. A click on a pointer that has rested longer than the delivery delay (about 1 ms) therefore lands on its recorded pixel; a click made while the pointer still moves lands within what the pointer travels in that delay: which move a report belongs to cannot be known more closely.
- If the current layout changes or becomes unknown while a playback that uses positions runs, the controller stops it and says why. A relative-only playback carries on.

**KWin scripts.**
- Two scripts, embedded in the binary, each under one fixed plugin name. The layout script stays loaded while the daemon runs and calls it once at load and on every layout change, never on pointer motion. The position script is loaded only while a recording runs, and is unloaded however the recording ends.
- Every load first unloads its plugin name. At start the daemon unloads both names and sweeps its script files, so nothing a crashed daemon left stays; it does both only after it has claimed the single-instance name, and a process that fails to claim it touches neither. At exit it unloads both.
- An unload that fails is remembered and tried again before the next load and at exit.
- When `org.kde.KWin` gets a new owner the daemon loads the layout script again, and the position script if a recording runs.
- Without KWin on the bus nothing is loaded, nothing is started through D-Bus activation, and recording and playback work as in R02.

## Decisions

Decided by the owner (2026-11-10):
1. **Changed layout:** positions stay in use when the outputs the recording's positions lie on are unchanged; otherwise relative motion, with a tooltip. No refusing and no rescaling.
2. **The layout script stays loaded** for the daemon's lifetime. Decision 0001 describes one script, loaded only while recording, that reports positions and the layout size; that now describes the position script, and the layout comes from the second one.

Defaults (reviewed; go ahead without asking):
3. **D-Bus:** zbus 5, already a direct dependency. The receiver is an interface on the daemon's existing connection and name (`io.github.tobylo.AutoClicker`); no second name.
4. **Script files:** a private file (0600, created exclusively, no symlink followed, a unique file name per load) in a directory of the daemon's own under `$XDG_RUNTIME_DIR`. The scripts are loaded only when that directory is owned by the user and closed to everyone else, checked on the handle the file is created through.
5. **Trust:** a call is accepted only when its sender is the current owner of `org.kde.KWin` and it carries the token baked into the script at that load. This guards against stale scripts and stray callers. It is not a boundary against other processes of the same user, which can load KWin scripts of their own (decision 0001).
6. **Stored data:** logical positions, and per output its rectangle and scale, without names. Scale is stored, not compared: logical positions do not depend on it. Decision 0001's "playback must compare every output's rectangle and scale" is replaced by owner decision 1.
7. **Format:** version 2 is version 1 plus a `layout` field and an `"at"` event, under I1 and I2. A recording has either both a layout and at least one position (version 2) or neither (version 1, as written today, so older builds still read it). Versions 1 and 2 are read; a version 1 file with `layout` or `"at"` is refused. Version 2 is fixed when R03a merges: a later change takes a new version number.
8. **Skip rule:** the 5 ms window, with presses, releases and wheel turns kept after their position, as under "Playback". The file keeps raw moves and positions, so the rule can change without re-recording. The two errors are not alike: a desktop move wrongly sent is corrected by the next position, while a game move wrongly skipped (within 5 ms before an unlock) is a small lasting view error.
9. **Where it runs:** zbus handlers must be `Send` and run in order (no task per call), and the recording slot lives on the controller's thread. The handler stamps the call and passes it on; a watcher polled on the daemon's thread beside the controller feeds the recorder through the existing `RecordingFeed`, woken by its change counter and by calls, never by a timer. The controller remains the only one that starts or stops a recording.
10. **Layout check at play start** is one comparison against the current layout (I4): no new wait before playback.

## Slices

| Slice | Holds | Size |
| --- | --- | --- |
| R03a: positions in the core (#51) | Format version 2; the recorder takes the current layout and positions; tray trim; playback's plan (absolute moves, covered moves, presses and wheel turns after their position, the layout rule); `Platform` gives the current layout (unknown by default); the controller's tooltip and its stop on a layout change. Tested against fakes only | about 1,400 lines |
| R03b: positions from KWin (#52) | New crate `backend-kwin`: the two scripts, loading and unloading, the receiver, the watcher; daemon wiring, cleanup at start, unload at exit; Plasma checks; README, map (D3, D7) and decision record | about 1,400 lines |

R03a comes first and changes nothing a user can see: no platform reports a layout yet, so no recording gets positions.

## Acceptance criteria

R03a, each a test against the fake backend:
1. A version 1 recording plays exactly as before, with nothing new in the tooltip.
2. A recording with positions plays them as absolute moves at their times, does not send covered moves and sends the others.
3. A press, a release and a wheel turn between a move and the position covering it are each played after that position; a position before one of them survives a later one in the same millisecond.
4. A layout that moved or lost an output the recording used falls back to relative motion and says so; an added output beside unchanged ones does not; an unknown layout falls back.
5. The current layout changing or becoming unknown during a playback that uses positions stops it at once, with nothing pressed (time-bounded).
6. Format: version 2 round-trips; a recording without positions is written as version 1; the refusals of decision 7, I1 and I2 each have a fixture.
7. Recorder: events come out in time order when an input event is read late; one position per millisecond, with the first after a press, release or wheel turn kept; the first position written at the start when nothing moved before it; no positions while the layout is unknown or differs, and again once it matches; an idle recording saves nothing; the tray trim takes its positions.
8. I3: reports the recorder must drop never fail a recording, and the longest recording the recorder writes, with positions, loads.

R03b:
9. A call from anyone but KWin's owner, or with a stale token, is ignored (test).
10. Loading and cleanup with a fake KWin (test): scripts loaded and unloaded as "KWin scripts" says, on every way a recording ends; stale scripts and files removed at start, only after the name is claimed; a failed unload retried; scripts loaded again after a new owner; nothing loaded and nothing activated without KWin; a runtime directory open to others loads nothing.
11. The same on Plasma, by hand: nothing left after exit, and after a crash the next start cleans up.
12. Recorded clicks on desktop targets replay within 1 px per axis on one and two monitors (manual check on Plasma, numbers in the PR).
13. A recording that goes from the desktop into a pointer-locked game replays both parts (manual check on Plasma, with the measured delivery delay of a real mouse against the 5 ms window).
14. G2's "Record → play round trip" is ticked once criteria 12 and 13 pass.

## Open questions for the slices

R03a:
- How I3's size bound is met: the recorder stops and saves before its file would pass `MAX_FILE_BYTES`, as it does at its time limit, or the cap moves. An hour of moves and positions at one each per millisecond is about 245 MB against 256 MiB; the size notes in `library.rs` and `controller/playback.rs` change with it.
- The delivery delay within which a position after a press, release or wheel turn is taken to belong before it: about 1.2 ms measured on synthetic input (decision 0001), to be fixed with a margin and checked against a real mouse in R03b. Fixtures: a move that produces no report (the pointer against an edge), then a press, then a move away with its position, inside and outside the delay.
- Where a late input event is inserted among positions already written, and how that meets the motion bucket (a bucket's move is written before a position stamped after it).
- Whether `Change` gains a variant or positions get a type of their own; find every place that matches on moves by search (`trim.rs` compiles either way).
- The stored number type of a rectangle (KWin's logical sizes, 2293 where KScreen says 2294), and checked arithmetic in the formula.
- P02 (#20) changes `Platform` and the controller at the same time: rebase before the PR, and name a collision instead of guessing.

R03b:
- Whether the script file can be removed as soon as `run()` returns (check on the laptop); KWin opens it by path.
- A stop and a start between two polls of the watcher must give the new recording its own initial position (key on the slot's change counter and the load token). The token is random hex, unique across daemon runs.
- The hand-off from the handler to the watcher keeps every report, in order: playback's pairing of clicks and positions depends on none being replaced. How it stays bounded when the daemon's thread stalls (positions stop until it is drained, leaving a gap) is this slice's to settle; sender and token are checked before anything else; the cached owner is read after subscribing to `NameOwnerChanged`.
- A layout-change call that never arrives leaves the current layout stale; whether to reconcile it, and when.
- The monotonic clock for stamping without a second `unsafe` copy of `backend-kernel`'s.
- What KWin does with an absolute move while a game holds the pointer (X01 sent relative moves only), and which output route criterion 12 is measured on (the portal route maps positions through its own regions).
- Whether the user service restarts after a crash, so a stale position script does not keep costing KWin.

## Needs the owner

- The laptop's user is not in the `input` group, so the laptop cannot record. Criteria 12 and 13 need a way to record there (group membership or another route), which is the owner's to choose and do.
- A second monitor attached to the laptop for criterion 12.
