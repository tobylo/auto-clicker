@AGENTS.md

## Claude Code and T3

AGENTS.md, imported above, holds the rules for every harness. These add what is specific to Claude Code and T3.

- **Runs in T3.** You run as a T3 Code thread on the dev box, in full-access mode: nothing waits for a permission prompt, so you own every action you take. When you need the owner, ask with `AskUserQuestion`: T3 pushes it to the owner's phone and keeps the thread waiting for the answer. Your final message reaches the owner the same way. Start other threads only as "Handing over" or "Review in a fresh session" says, with the `start-slice-session` skill's launch settings ([docs/agents/slice-workflow.md](docs/agents/slice-workflow.md)).
- **Shared skills name devisor.** `start-slice-session` and `pick-next-work` were written for devisor: use this repository's main checkout (`/home/tobylo/.t3/projects/auto-clicker`), its T3 project and Project 4 wherever they name devisor's, and skip their Entire steps.
- **Rust toolchain.** rustup in `~/.cargo`; `cargo` is on PATH in fish through `~/.config/fish/conf.d/rustup.fish`. In other shells, `source ~/.cargo/env` first.
