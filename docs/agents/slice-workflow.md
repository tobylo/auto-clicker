# Slice, planning and review sessions

Work on this repository's board runs as one session per piece of work: a feature is refined in a planning PR when it needs one, then each slice is implemented in its own session and reviewed in a fresh one. These rules add to [AGENTS.md](../../AGENTS.md) for those sessions. The steps that launch a new session use Claude Code and T3 (the `start-slice-session` skill, `t3_thread_launch`); other harnesses stop at that step and tell the owner what to start.

## Docs in step
- Settle each slice's open questions in the issue or its brief, each with its test.
- Close the follow-up duties other issues put on yours, and tick their criteria on the owning issue.
- Update [docs/planning/map.md](../planning/map.md) when a slice or spike changes a decision recorded there.
- An investigation ends with a short decision record in `docs/decisions/`, numbered in sequence, with the evidence (commands, versions, results) that supports it.

## Planning PRs
- Before the first question, have one fresh reviewer agent rate your proposals, as the `grill-feature` skill's review step says, and check each challenge against the code. Then send a plain-language briefing of the work, with its links, in a message that ends your turn, and wait for the owner's reply: text above an open dialog does not reliably reach the owner, and links in a dialog cannot be clicked. The briefing lists the low-stakes defaults as going ahead unless the owner objects. Architecture, ongoing burden, anything hard to undo and anything the reviewer challenged come as questions, one at a time, through the dialog, each readable on its own: which work, which question of how many, what is being decided, and what each answer changes, in plain words. Propose an answer, then ask only what is genuinely open.
- Size each slice to about 1,500 added lines, tests included: review rounds grow with PR size. Where a slice cannot be that small, say why in the breakdown.
- Planning documents get one deep adversarial round and at most 2 local rounds. Fix decision-level findings, and route implementation-level findings to the owning slice's open questions.
- Open the PR with the feature's criteria mapped to slices as a checklist, written as "The PR" says. Comment `@codex review`, answer every bot finding, and merge once it's green.
- Once it has merged, check the branch is gone and fast-forward the main checkout, then hand over to the first slice's session (below). Implement nothing in the planning session.

## Review in a fresh session
This session implements the slice; a new session reviews it and carries it to its merge. Every turn re-reads the session's context, and by review time that context is large.

Implement the slice, commit it with the gates passing, and review it yourself once. Then stop before the first review round: run no reviewers here.

1. Write <main checkout>/.claude/local/<slice>-review-handoff.md, about 60 lines at most:
   - the branch and its HEAD, this slice's prompt file and the path of this worktree;
   - what you decided that the issue and the prompt do not state, with the reason, including the owner's answers in the dialog;
   - what you tried and rejected, and why;
   - what is out of scope on purpose;
   - each new guard and the test that fails without it;
   - which `#[ignore]`d device tests you ran by hand, and their result;
   - where you are least sure.
2. Release the branch with `git switch --detach` in your slice worktree, so the new session can check it out. Do not push it, and remove nothing.
3. Launch a T3 thread with `t3_thread_launch`, with the model and runtime mode the `start-slice-session` skill gives, title `<slice> review — <issue title> (#<issue>)`, `workspaceStrategy` `{"type": "root"}`, and this prompt as its message, with the paths filled in:

   > You take over slice <slice> (#<issue>) from its first review round to its merge and handover. The implementation is done and committed. AGENTS.md and docs/agents/slice-workflow.md apply to you; read the slice's prompt file whole for its own sections. Its "Review in a fresh session" is already done. Do not implement anything beyond what review findings require. Then read the handoff file, which holds what the first session decided. From the main checkout, run `git worktree add .claude/worktrees/<slice>-review <branch>`, work there, and run the gates as AGENTS.md says. Read the diff whole for the self-review before each round, and other code only as a finding needs it. Run the adversarial-review skill from its first round. After merging, remove both worktrees: yours and the implementation worktree the handoff names.
4. Check with `t3_thread_read` that its run is running, tell the owner in two lines which thread now carries the slice, with its link, and stop.

## Handing over to the next session
Use the variant that fits the work.

**Any merged PR with unblocked work left on the board:** after the PR merges, start the next unblocked issue's session with the `start-slice-session` skill: the highest priority first (P0, then P1, then P2), then the lowest issue number. The one exception is a merged planning PR, which starts its own feature's first slice, so a refined feature is carried through before other work. Skip an issue a running T3 thread already carries, and an issue whose work would change the same crate or files as a running session's issue (two sessions changing the controller, say); take the next one instead. Keep two issue sessions running: count the running T3 threads titled `… (#<n>)`, then start sessions until two run, or until nothing suitable is left. Normally that is one per merge. Then tell the owner in two lines what merged and which thread now carries the next slice, with its link. Then stop. Do not implement the next slice yourself, and do not wait for it.

**The last slice of a milestone:** after the PR merges, check the branch is gone and the issue closed with its card Done, and fast-forward the main checkout. Tick the milestone scope gate's remaining criteria once their tests pass on the merge commit, and close it if every box is ticked and every child closed. Then start the next unblocked issue as above.

**A standalone task or an investigation:** after the PR merges, or once the evidence is recorded, check the branch is gone and the issue closed, then start the next unblocked issue as above.

**Nothing unblocked left:** start no session. Tell the owner in two lines what merged and what is waiting on them, if anything.
