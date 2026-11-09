# Working in this repository

These rules apply to every agent session in this repository, whatever the harness. Slice, planning and review sessions also follow [docs/agents/slice-workflow.md](docs/agents/slice-workflow.md). "The owner" is the repository's owner. How to do a step in a particular harness (launching threads, question dialogs, watching a PR) lives in that harness's own configuration. Where a rule names a Claude Code or T3 tool or path, another harness uses its equivalent, or reads the named file.

What the product is and what has been decided: [docs/planning/map.md](docs/planning/map.md).

## Design for these up front
- **One owner of state.** The daemon has one controller that owns the mode (idle, clicking, recording, playing). The tray, hotkeys, the settings window and timers send it messages; none of them changes state directly. A new path must not add a second writer.
- **Every wait has one shape:** select on the wait, the controller's stop signal and shutdown. A stop ends it at once. Give every "stops at once" test an upper time bound well under the wait it interrupts, or a mutant that ignores the stop passes, only slower.
- **Nothing stays pressed.** Every exit path (stop, error, panic, backend loss, shutdown, signal) releases every button and key the program pressed. A stuck mouse button or key in a game is the worst failure this program can have.
- **The program never reacts to itself.** Events from its own virtual devices are never recorded, never read as hotkeys and never counted as user input. Identify them by device identity, not by name.
- **Idle costs nothing.** No polling loops and no busy waits. Timers use absolute deadlines (`timerfd` or the async runtime's sleep-until), and the daemon sleeps on file descriptors when idle.
- **Backends behind one interface.** Engine, recorder, hotkeys and tray talk to the input backend trait only; nothing outside a backend module touches `/dev/uinput`, evdev, libei or the portals.
- **Recurring findings mean a redesign.** If one layer keeps producing findings round after round, stop, analyse the shared root cause, and ask the owner with a PR Lens canvas before rewriting.
- **Review your fixes like the change.** Most findings after round 1 are in code written after round 1, mostly the fixes themselves. Before committing a fix, put its own diff through the checklist below, find every other site with the same pattern (`rg`) and fix them in the same commit, or move the guard into the layer that owns them all. Prefer the fix that removes a mechanism to one that adds a check, a retry or a cleanup path.
- **Check your diff for these before round 1 and before each fix commit.** They are the defect families behind most review findings:
  - *The checked thing is the used thing.* Keep the handle or device you validated and act on it; never re-open by name or path after a check.
  - *Untrusted input is accepted in one form.* Config files, recordings and D-Bus replies are parsed once into a validated type; reject what does not parse rather than filtering bad spellings.
  - *Every exit leaves nothing behind.* For each step of a multi-step operation, say what remains if it fails, including when the cleanup fails: virtual devices, grabbed devices, portal sessions, pressed keys, temp files.
  - *Every transition reaches every member.* Stop, pause, backend loss and shutdown apply to every task, timer and device a change adds: bind them to their owner's cancellation.
  - *Recorded state tells the truth after a crash.* Write a recording or config atomically (write a temp file in the same directory, fsync it, rename, fsync the directory), never a half file.
  - *Errors keep their meaning.* Tell transient from permanent (a device unplugged versus permission denied), and never read a failed or empty read as "nothing there".
  - *Every wait is bounded.* Each wait has its own deadline, cleanup calls included. The one exception is the daemon's idle wait for events, which may block indefinitely but must end at once on shutdown.

## Hard rules
- No `unsafe` outside a backend module, and every `unsafe` block carries a `// SAFETY:` comment.
- No `unwrap()`/`expect()` on runtime paths; tests and provably infallible cases (with a comment) only.
- Check arithmetic on counts, intervals and durations for overflow (`checked_*`, `saturating_*`, or a typed duration).
- Plasma on Wayland (6.8 and later) is the primary target. X11 sessions are not a target; X11 apps under XWayland are, since games run there.
- Never log or persist keystrokes outside an active recording the user started.

## How we work
- **Failures get fixed or taken up with the owner.** Anything you see fail that ought to be looked at is yours once you see it: a tool, script, CI job, test or review step, whether or not it touches your task. A warning, a fallback or a round that carries on does not close it.
  - Look into it read-only for a few minutes, then fix it if the fix is small and yours to make, or tell the owner at the top of your next message: what failed, since when, what it affects, the cause as far as you found it, and what you recommend.
  - Don't run a destructive repair just because an error message suggests it.
  - A subagent puts it first in its report, and its parent passes it on to the owner. If it is still open, say so again in your final message.
  - Expected failures don't count: the test you are fixing, or a job whose message says it is waiting.
- **Branches.** One branch per PR, from origin/main, each in its own worktree: the one T3 created for your thread, or one under .claude/worktrees/.
- **Gates.** Commit only after `scripts/gates.sh` reports that all gates passed (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`). Do not edit the tree while the gates run, and commit exactly the tree that passed. After rewriting a test table, compare its case names with the previous commit: a rewrite can drop rows.
- **Tests that need devices.** Unit tests never need `/dev/uinput`, an input device or a running compositor: they run against a fake backend. Tests that drive real devices are `#[ignore]`d with a reason, run by hand (`cargo test -- --ignored`) on a Plasma session, and the PR says when they last passed and on which Plasma version.
- **Timing in tests.** Assert outcomes and orders rather than elapsed times; use time bounds only as generous upper limits. CI runners are slow and noisy.
- **Code Health.** Every source file a commit adds or changes that CodeScene scores (Rust, scripts) scores 10; documentation and data files are exempt. Before pushing, also run `analyze_change_set` against origin/main. If a file can't reach 10, ask before committing.
- **Reproducible state.** Keep review clones, calibration lists and mutation scripts under the worktree's .claude/local/, which is git-excluded.
- **Scope.** Do not widen an issue's scope on your own: propose a separate issue for work outside it. Build the lasting solution from the start; write no interim code you already know will be replaced.
- **Dependencies.** Prefer few, well-maintained crates. Assess what a crate or release adds against this program's needs (binary size, idle cost, build time), alongside compatibility, and verify its claims by running them.
- **Upstream reports.** For an issue in a dependency, KWin or a portal, give the owner the facts: a minimal reproducer, observed against expected, the `file:line` in the upstream source, where it hurts in practice, and a check that it is not already reported. The owner writes and files the report.
- **Local review.** Before pushing, run the branch review in `~/.claude/skills/adversarial-review/SKILL.md` on the dev box, in any harness, with `REVIEW_ENTIRE=0 REVIEW_ENTITY_MAP=0` (this repository does not use Entire). Its "Stopping rules and budgets" set the self-review before each round, what each re-run covers, the full scan and the round limits. It covers the Codex lenses and the Codex generic review, once per PR (launch both only through its `run-lenses.sh`), and CodeRabbit in every full scan after the first, from the repository root. Add:
  - mutation testing (`cargo mutants` on the changed files, `--in-diff`), run by this session and never by a subagent, started once in the background beside the round's lenses, with its results in a file. A stop or release mutant needs a time-bounded test. Once a full scan is clean, a surviving mutant or a minor test gap alone does not extend the review;
  - a Claude pass over the guards and tests, given those results, in the first round, and in later rounds over the guards the fixes added: for each new guard, name the test that fails without it, and report a guard with none.

  Report each round's Codex "tokens used". A wrong design is a rewrite the owner decides. Decline a finding that reopens a settled decision, with the reasoning.
- **The PR.**
  - Open it with "Closes #<slice issue>" (and "Closes #<feature issue>" on the last slice) and, for a change with moving parts, a PR Lens canvas link and its top view.
  - Give it an "Acceptance criteria" section: the issue's criteria and validation list, as checkboxes, each naming the test or doc section that proves it. Tick each only after re-running those tests on the head commit.
  - Write the title and description as the owner would as the PR's author: fluent, plain and to the point.
    - Write for a reader who knows what the program is for but not its internals. Lead with the effect, and let internal names support the sentence rather than carry it.
    - Open with one or two sentences: what changes, and why.
    - State each design choice as the change's own, with its reason. Never attribute it: no "owner decision" or "settled with the owner". Who decided what belongs in the map or the brief; link it where a reviewer needs the history.
    - "What changes" covers the behaviour a reviewer can check, in a few bullets. Name a file or type only where it helps someone read the diff.
    - Leave the process out: no session history and no log of each review round. "Local review" takes a few lines: what ran, the mutation result, and any declined finding a reviewer may raise again, with the reason.
    - Plain sentences: no filler, hype or hedging, no bold lead-in on every bullet, and nothing said twice.
    - The same voice holds for issues, commit messages and anything else others read. None of them carries attribution or co-author lines or mentions the AI agent.
  - Comment `@codex review` on the PR's first head, and again only after a push that changes its code. Answer every bot finding (Codex and CodeRabbit): "fixed in <sha>", or the reasoning.
  - **Wait by watching, not polling.** Right after opening the PR, start the harness's PR watch where it has one (T3: `watch_pull_request`). Whenever you are waiting on bots or CI, end your turn; handle what wakes you, then end the turn again. A run that stays queued or in progress with no wake is a stall.
  - At most 5 bot rounds; then report to the owner with a recommendation.
- **Ready means green.**
  - Never call a PR ready until `gh pr checks` is green on the head commit and every bot finding is answered.
  - Merge by hand once that holds, never with `gh pr merge --auto`. The owner has authorized every session to merge its own PR once it is green and to move on to the next unblocked issue without asking (2026-11-09). Ask only when a decision is genuinely the owner's, or something needs their hands (their machine, a GitHub app, a credential).
  - A review bot that has not responded within 30 minutes of the PR's first head is treated as not installed: say so in the PR and in your final message, and go on with the local Codex and CodeRabbit review alone.
  - `main` requires a branch to be up to date: when main moves, update the branch and let CI run again before merging.
  - A test that fails once in CI is reproduced locally (`cargo test` repeated, under load) before it is called a flake, and fixed at its cause.
- **After merging,** check the branch is gone, fast-forward the main checkout (`git -C <main checkout> pull --ff-only`), remove your worktree and local branch once nothing in them is needed, and check the issue is closed and its card Done. Then hand over as [docs/agents/slice-workflow.md](docs/agents/slice-workflow.md) says under "Handing over".

## Working with the owner
- **The owner often reads from a phone or another machine.** Put what they need in the message itself: file contents, results, links. They cannot open a path on your machine. When an explanation needs a picture, draw it (a PR Lens canvas, or an SVG file sent to them) instead of offering to.
- **Commands for the owner's host** work in fish, the owner's shell: one line, absolute paths, or a single `sudo bash -c '…'`. Multiline pastes get mangled.
- **Keep going on authorized work.** Ask only when the answer changes what you do, and stop when the work is done. While waiting on something long, say what you are waiting for: silence looks stuck.
