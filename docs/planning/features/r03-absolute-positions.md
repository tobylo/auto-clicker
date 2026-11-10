# R03: Absolute cursor positions in recordings

- Issue: [#16](https://github.com/tobylo/auto-clicker/issues/16), a Feature under [G2 (#3)](https://github.com/tobylo/auto-clicker/issues/3), milestone M2. Slices: R03a (#51), R03b (#52).
- Maturity: refined 2026-11-10, ready to slice. Rests on [decision 0001](../../decisions/0001-cursor-position.md) (X01) and map D3, D4, D7, D11.
- Diagram: [the flow from KWin to the file and back](https://prlens.dev/c/SSdbz2qLBRoW1UnAP_2jtw).

## Problem and outcome

A recording stores relative motion only. Replayed on the desktop it passes through pointer acceleration, so 400 units land 397 to 795 px away depending on speed (R02). With R03, a recording made on the desktop replays its clicks to the pixels they were made at, within 1 px per axis at scales up to about 1.8 (decision 0001). In a game that locks the pointer nothing changes: relative motion is recorded and replayed.

## Scope

In scope: reading the pointer position and the output layout from KWin while recording; storing both in the recording; replaying positions as absolute moves; falling back to relative motion; loading, unloading and cleaning up the KWin scripts.

Out of scope: sub-pixel positions (upstream wish 3 in decision 0001); rescaling positions to another layout; positions on compositors other than KWin; recording through the portal (P03); replacing the Ctrl+F7 hotkey (map, open questions).

## Behaviour

**Recording.**
- While a recording runs, a KWin script reports `workspace.cursorPos` (whole logical px) once at load and on every change that differs from the last one sent. The daemon stamps each report on the event clock when it arrives.
- The recorder writes positions as events of their own, at most one per millisecond (the latest wins), beside the relative moves it already writes.
- Events are ordered by time, not by arrival. A report is never stamped earlier than the move that caused it, so an input event read late goes before the positions stamped after it, where today it would be moved forward.
- A recording has one layout: the one current at its start. If the layout is unknown then (no KWin) or changes during the recording, no positions are written from that moment on. A recording with no positions is written as today.
- An idle recording still saves nothing: positions alone do not make a recording.
- The trip to the tray is trimmed as today; positions inside the trip go with its motion.

**Playback.**
- Positions are used when every output a recorded position lies on is present in the current layout with the same rectangle. A monitor added beside an unchanged screen therefore keeps positions exact; a screen that moved, changed size or is gone does not.
- Otherwise (another layout, layout unknown, version 1 file) the whole run uses relative motion as R02 does, and the tray tooltip says why while it plays.
- When positions are used, each one is sent as an absolute move to `clamp(round((x - left) * 65536 / W), 0, 65535)` (likewise y), with W, H and the origin of the current layout's bounding box. A relative move is skipped when a position follows it within 5 ms in the recording; every other relative move is sent (a locked pointer, KWin restarting, the pointer pushed against an edge).
- If the layout changes while a playback that uses positions runs, the controller stops it and says why. A relative-only playback carries on.

**KWin scripts.**
- Two scripts, embedded in the binary. The layout script stays loaded while the daemon runs and calls it once at load and on every layout change, never on pointer motion. The position script is loaded only while a recording runs.
- At start, after the single-instance name is claimed, the daemon unloads any script of its own a crashed daemon left. At exit it unloads both, within a bounded time. When `org.kde.KWin` gets a new owner it loads them again; the stretch in between has no positions.
- Without KWin on the bus nothing is loaded, nothing is started through D-Bus activation, and recording and playback work as in R02.

## Decisions

Decided by the owner (2026-11-10):
1. **Changed layout:** positions stay in use when the outputs the recording's positions lie on are unchanged; otherwise relative motion, with a tooltip. No refusing and no rescaling.
2. **Layout script stays loaded** for the daemon's lifetime. This amends decision 0001's "a daemon that is not recording has no script in KWin", which now holds for the position script only.

Defaults (reviewed; go ahead without asking):
3. **D-Bus:** zbus 5, already a direct dependency. The receiver is an interface on the daemon's existing connection and name (`io.github.tobylo.AutoClicker`); no second name.
4. **Script files:** written to a private file (0600, created exclusively, no symlink followed, a unique name per load) under `$XDG_RUNTIME_DIR/auto-clicker/`, and removed again; leftovers are swept at start.
5. **Trust:** a call is accepted only when its sender is the current owner of `org.kde.KWin` (cached, kept current from `NameOwnerChanged`) and it carries the token baked into the script at that load. This guards against stale scripts and stray callers; any session process can still load a KWin script of its own (decision 0001).
6. **Stored data:** logical positions, and per output its rectangle and scale, without names. Scale is stored but not compared: logical positions do not depend on it.
7. **Format:** version 2 is version 1 plus a `layout` field and an `"at"` event. Versions 1 and 2 are read. A recording without positions is written as version 1, so it stays readable by older builds. A version 1 file with `layout` or `"at"` is refused; in version 2 every position lies inside the layout's bounding box and the number of outputs is bounded.
8. **Skip rule:** the 5 ms window above. The file keeps raw moves and positions, so the rule can change without re-recording. The two errors are not alike: a desktop move wrongly sent is corrected by the next position, while a game move wrongly skipped (within 5 ms before an unlock) is a small lasting view error.
9. **Where it runs:** zbus handlers must be `Send` and must run in order (no task per call), and the recording slot lives on the controller's thread. The handler stamps the call and passes it on; a watcher polled on the daemon's thread beside the controller feeds the recorder through the existing `RecordingFeed` and follows its change counter, as the evdev reader does. The controller remains the only one that starts or stops a recording.
10. **Layout check at play start** is one comparison of values the daemon already holds: no new wait before playback.

## Slices

| Slice | Holds | Size |
| --- | --- | --- |
| R03a: positions in the core (#51) | Format version 2; the recorder takes a layout and positions, ordered by time; tray trim; playback's plan (absolute moves, skipped relative moves, the layout rule); `Platform` gives the current layout (none by default) and the controller says so when it falls back and stops a positions playback when the layout changes. Tested against fakes only | about 1,400 lines |
| R03b: positions from KWin (#52) | New crate `backend-kwin`: the two scripts, loading and unloading, the receiver, the watcher; daemon wiring, stale-script cleanup, teardown; Plasma checks; README, map, decision record | about 1,400 lines |

R03a comes first and changes nothing a user can see: no platform reports a layout yet. No version 2 file exists before R03b, so R03b may still adjust the format after its first real recording.

## Acceptance criteria

1. Recorded clicks on desktop targets replay within 1 px per axis on one and two monitors (R03b: manual check on Plasma, numbers in the PR).
2. A version 1 recording plays exactly as before (R03a: test against the fake backend).
3. A recording with positions plays them as absolute moves at their times, skips covered relative moves and sends uncovered ones (R03a: test).
4. A layout that moved or lost an output the recording used falls back to relative motion and says so; an added output beside unchanged ones does not (R03a: test).
5. A layout change during a playback that uses positions stops it at once, with nothing pressed (R03a: time-bounded test).
6. The longest recording the recorder can produce, with positions, still loads (R03a: test against `MAX_FILE_BYTES`).
7. A call from anyone but KWin's owner, or with a stale token, is ignored (R03b: test).
8. The scripts are loaded and unloaded by the daemon and nothing is left on exit; after a crash the next start unloads the stale scripts and sweeps the script files (R03b: test with a fake KWin, and a manual check on Plasma).
9. A recording that goes from the desktop into a pointer-locked game replays both parts (R03b: manual check on Plasma, with the measured delivery delay of a real mouse against the 5 ms window).
10. G2's "Record → play round trip" is ticked once criteria 1 and 9 pass (R03b).

## Open questions for the slices

R03a:
- Where a late input event is inserted among positions already written, and how that meets the one-per-millisecond rule and the motion bucket (a bucket's move is written before a position stamped after it).
- Whether `Change` gains a variant or positions get a type of their own; every place that matches on moves must be found by search (`trim.rs` compiles either way).
- How the layout reaches a recording that starts before the daemon has heard from KWin.

R03b:
- Whether the script file can be removed as soon as `run()` returns (check on the laptop); that would remove the cleanup at unload.
- The watcher keys on the slot's change counter and the load token: a stop and a start between two polls must give the new recording its own initial position.
- The monotonic clock for stamping without a second `unsafe` copy of `backend-kernel`'s.
- What KWin does with an absolute move while a game holds the pointer (X01 sent relative moves only).
- Whether the user service restarts after a crash, so a stale position script does not keep costing KWin.
- P02 (#20) changes `Platform` and the controller at the same time: rebase before the PR, and name a collision instead of guessing.

## Needs the owner

- The laptop's user is not in the `input` group, so the laptop cannot record. Criteria 1 and 9 need a way to record there (group membership or another route), which is the owner's to choose and do.
- A second monitor attached to the laptop for criterion 1.
