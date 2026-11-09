# Planning tracker conventions

GitHub owns live status. This file records how planning work maps onto the tracker, so that planning tools and people apply the same metadata.

## Tracker

- **Repository issues** in `tobylo/auto-clicker`; no Project drafts.
- **One Project:** [Auto Clicker — Delivery](https://github.com/users/tobylo/projects/4) (user project 4). Every issue is on it.
- **Milestones** carry the delivery scope: M0 Foundation, M1 Auto clicker, M2 Record and playback, M3 Settings window, M4 Portal backend. Version 1.0 is M0 to M4. Each milestone has a scope-gate issue (Kind Epic) that lists its tasks and its exit criteria.
- **Decomposition** uses native sub-issues. A milestone's tasks are sub-issues of its scope gate. A task later refined into slices becomes a Feature and its slices are its sub-issues.
- **Blocking** uses native `blocked_by` relations, repeated as a `Blocked by:` line in the body. Parentage never implies blocking.
- **Follow-up duties.** When one issue gives another a duty (a spike's result changes a task, say), the duty becomes an acceptance criterion in the owning issue's body, not only a comment.
- **Stable IDs.** Every issue title starts with its planning code (`S01`, `X01`, `C02`, …). [github-map.json](github-map.json) maps codes to issue numbers.
- **Publication.** Planning sessions may create and update issues and Project items for settled scope without asking again.

## Project fields

| Field | Values | Rule |
| --- | --- | --- |
| Status | Todo, In Progress, Done | Live state |
| Kind | Epic, Feature, Task, Investigation, Bug | See below |
| Priority | P0, P1, P2 | From the body's metadata line |
| Size | S, M, L | From the body's metadata line |
| Uncertainty | Low, Medium, High | From the body's metadata line |

Kind:

- **Epic:** a milestone scope and exit gate.
- **Feature:** a task that has been refined and split into slices. It keeps its issue number and gains Task children.
- **Task:** deliverable implementation work.
- **Investigation:** work whose deliverable is evidence or a decision rather than product code: spikes and feasibility proofs. It ends with a decision record in `docs/decisions/`.
- **Bug:** a defect in existing code.

## Labels

| Label | Covers |
| --- | --- |
| `area:repo` | Repository tooling, gates, CI, agent docs |
| `area:input` | Input backends: uinput/evdev and portal/libei |
| `area:engine` | Clicker engine and scheduling |
| `area:recording` | Recording format, recorder and playback |
| `area:hotkeys` | Global hotkeys |
| `area:tray` | System tray icon and menu |
| `area:ui` | Settings window |
| `area:packaging` | PKGBUILD, udev, systemd, install docs |
| `area:platform` | Compositor and desktop integration (KWin, portals) |

## Issue body

```
Kind: Task · Priority: P1 · Size: M · Uncertainty: Low
Blocked by: #n, #m

## Outcome
## Scope
## Out of scope
## Acceptance criteria
- [ ] …
## Validation
```
