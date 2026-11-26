# The Windows test PC

Besides the VM ([windows-vm.md](windows-vm.md)), sessions run Windows device tests on the owner's Windows 11 PC, `win11-test`, which has real hardware: no hypervisor, a real graphics card, fullscreen (WN13, brief decision 17). `scripts/windows-pc/pc` reaches it from the dev box the way `vm` reaches the VM. It uses the same desktop runner, the same `run`, `ssh` and `push`, and the same deadlines and exit codes. The PC is a real machine on the LAN, not a VM: no command shuts it down, restarts it, resets it or presses keys in it.

## What it is

- **Windows:** Windows 11 Pro, build 26300 and its updates, in Swedish. `pc status` prints the exact build; a PR that ran tests there names it.
- **Hardware:** Intel UHD graphics, a 1920×1080 display at 125 %.
- **Accounts:** the local user `tester` is an administrator with UAC on.
  - It logs in automatically at boot. Its password is random, made by `pc setup` and kept on the dev box. Windows keeps it for automatic logon in its LSA secret `DefaultPassword`, which only administrators can read, so `tester` is used for nothing else.
  - The owner reads the password from the dev box to log in by hand.
  - SSH accepts only the dev box's key, from the dev box's address only (firewall rule `devbox-ssh`), and lands elevated, with no desktop.
- **Network:** the PC is on the LAN and reaches the internet like any other machine. Only SSH into it is limited, to the dev box.
- **Toolchain:** Visual Studio Build Tools 2022 (the MSVC linker and a Windows SDK) and Rust stable with clippy and rustfmt, for `tester`.
- **Power:** it never sleeps on AC power, so nothing has to wake it. The display turns off after a few minutes, which does not stop input injection or capture.

## Files on the dev box

Everything lives in `~/.local/share/auto-clicker-pc/` (mode 700; `WN13_STATE_DIR` changes it, for tests: commands from two state directories would not share a lock), outside the repository:

- `config`: where the PC is, kept off the repository. It holds two lines and nothing else but blank lines and `#` comments; `pc` refuses anything else:

  ```
  host=<the PC's address on the LAN>
  key=<absolute path of the dev box's private key for the PC>
  ```

- `known_hosts`: the PC's host keys, recorded under the name `win11-test`, so a new address needs only `host=` changed. Made once from the dev box's own entry:

  ```sh
  ssh-keygen -F <the PC's address> -f ~/.ssh/known_hosts | grep -v '^#' | awk '{ $1 = "win11-test"; print }' > ~/.local/share/auto-clicker-pc/known_hosts
  ```

- `password`: tester's password (mode 600). `pc setup` makes it when it is missing, before it changes anything on the PC. `pc uninstall` keeps it, since tester's password stays that one.
- `lock`: one command at a time, as for the VM.

## On the PC

- `C:\wn01`: the runner's files, copied from `scripts/windows-vm/guest/` (`runner.ps1`, `JobRunner.cs`, `run-job.ps1`, `click-test.ps1`), plus the PC's `screen.ps1` and `focus.ps1`. It also holds the queues, and `restore.json`, which records what a Windows setting was before the setup changed it (a power setting under the plan it belongs to), written to disk before the setting changes.
  - The runner's files and the queues are owned and written by administrators and SYSTEM only. `C:\wn01` is the setup's own: it makes the folder with those rules in one step, and refuses one that another account made or can write to, or a junction in its place, since nothing put inside by that account could be trusted. Remove such a folder from an elevated prompt, then run the setup again. `pc uninstall` leaves such a folder alone too, and reads nothing from it.
  - The unelevated runner writes its own queue's `ready`, log, `work` and `outbox`, and claims a job by moving it out of its inbox, which it cannot write.
  - So no other account can change the runner or hand it a job, and a job in the normal queue cannot hand one to the elevated runner. Programs running as tester are trusted as tester, elevated or not: Windows does not treat UAC as a security boundary, and the setup runs elevated over SSH from tester's own temporary folder and checks tester's Rust toolchain.
- Two scheduled tasks, `wn01-runner-normal` (unelevated) and `wn01-runner-high` (elevated), start the runners at tester's logon. The setup records, beside the runner, the definition it registered and Windows' export of each task, and registers a task again when either differs; it restarts a runner that started before its files were last installed.
- `C:\work`: where tests build and run. It is not the setup's: `pc uninstall` leaves it alone.

## Commands

All of them print to stderr as they go, and every wait in them has a deadline.

| Command | What it does |
| --- | --- |
| `pc setup` | Sets the PC up, or checks it is still set up: the runner and its tasks, automatic logon to `tester`, no sleep on AC, Build Tools and Rust. It changes only what is not already right and prints one line per item, ending with `setup: nothing changed` when all was right. Then it waits until both runners run (when tester is logged on). It needs SSH to work already. |
| `pc run [--elevated] [--timeout S] [--workdir DIR] -- COMMAND` | As `vm run`: runs `COMMAND` through `cmd.exe /c` in tester's desktop, prints its output and exits with its exit code: 124 at the deadline (the command's whole process tree is ended), 125 when the runner could not start it, 126 when the runner did not answer. |
| `pc screen [FILE.png]` | Captures the whole desktop at its real resolution to a PNG on the dev box, through the elevated runner, so it works while a test runs in the normal queue. It fails while the lock screen or a UAC prompt is in front. |
| `pc push PATH [DIR]` | Copies a local file or folder into `DIR` on the PC (default `C:\work`), as a tar stream over SSH. |
| `pc ssh [COMMAND]` | Runs PowerShell over SSH as tester, elevated, with no desktop. With a command, `pc` first lets scripts run in that session (Windows' default policy is left as it is: `Restricted`), sets its output to UTF-8 and puts tester's own PATH, cargo included, in place. |
| `pc selftest` | Checks that no elevated window is in front, that an injected click reaches a window, that a job past its deadline is ended with everything it started, and that the screen is captured. |
| `pc status` | Shows the Windows build, the boot time, automatic logon, who is logged on, whether the screen is locked, a pending restart, the sleep setting, the runners, and what has focus and whether it is elevated. It changes nothing. |
| `pc uninstall` | Removes what the setup added: both tasks and their processes, `C:\wn01`, and automatic logon (`AutoAdminLogon` off, the LSA secret and any `AutoLogonCount` or registry password removed). It puts back what `restore.json` recorded (password expiry, and sleep if the setup turned it off), checks that nothing is left, and names what it left alone: SSH, the toolchain, `C:\work` and tester's password. On a PC with neither a `C:\wn01` of the setup's own nor a runner task, nothing is the setup's, and it leaves automatic logon as it is. |

`pc run`'s output is read as UTF-8: a program that prints in the console's code page (850 on this Swedish Windows) shows non-ASCII letters, such as cmd.exe's own messages, as replacement characters.

## Running a test

```sh
scripts/windows-pc/pc status
# Copy the tree at HEAD into an empty C:\work\<your branch> (here wn13) and build it there.
git archive --format=tar --prefix=wn13/ HEAD | scripts/windows-pc/pc ssh "New-Item -ItemType Directory -Force C:\work | Out-Null; cmd /c 'rmdir /s /q C:\work\wn13' 2>\$null; if (Test-Path C:\work\wn13) { exit 1 }; cmd /c 'tar -x -f - -C C:\work'; exit \$LASTEXITCODE"
scripts/windows-pc/pc ssh 'cd C:\work\wn13; cargo build --locked; exit $LASTEXITCODE'
# Tests run in the desktop: some Windows backend tests need one.
scripts/windows-pc/pc run --timeout 1800 --workdir 'C:\work\wn13' -- cargo test --locked
# Device tests too, one at a time.
scripts/windows-pc/pc run --timeout 900 --workdir 'C:\work\wn13' -- cargo test --locked -- --ignored --test-threads=1
scripts/windows-pc/pc screen .claude/local/after-test.png
```

- Build over `pc ssh`, but run tests through `pc run`. Some of the Windows backend's unit tests show a window or keep the display on, and an SSH session has no desktop, so there they fail (2026-11-26: `the_popup_shows_the_latest_lines_takes_no_click_and_goes_whole` and `every_way_a_run_ends_leaves_no_thread_and_hands_throttling_back`).
- Use your own folder under `C:\work`, named after your branch. Other sessions' trees and logs are there too; leave them alone.
- Check `pc status` before a test that injects input:
  - With the lock screen up, injected input goes nowhere.
  - With an elevated window in front (an "Administratör: PowerShell" the owner left open, say), Windows drops every event an unelevated program injects. A click test then fails for that reason alone. `pc selftest` refuses to run its click then.
- The owner sometimes uses the PC. A window they leave open can take focus. If something is in the way that only their hands can clear (a locked screen, a dialog), ask them.
- A test that changes a Windows setting puts it back itself. No reset brings the PC back to a clean state.
- The PR names the build `pc status` prints. Measurements that the VM marks VM-only are measured here.
- Driving a game or a real mouse stays with the owner.

## When something fails

- **`pc` says the PC does not answer:** it is off, its address changed (update `host=`), or its host key changed (Windows reinstalled; record the new key in `known_hosts` only after the owner confirms it). Since the setup turns sleep off, it should not be asleep.
- **`pc setup` or `pc uninstall` says another one still runs on the PC:** a setup or uninstall the dev box gave up on (its deadline passed, or the session ended) goes on running on the PC, and holds a lock there until it ends. Wait for it, then run the command again.
- **The runners do not run** (`pc status` says so): run `pc setup`, which starts them when tester is logged on. After a restart they start at tester's automatic logon, about a minute after boot.
- **Look at the screen:** `pc screen`.
- **Logs on the PC:** `C:\wn01\queue\<queue>\runner.log` holds the runner's errors.
- **Windows Update** installs and may restart the PC on its own; `pc status` shows a pending restart. After one, tester logs in again and the runners start, but a test running at that moment is lost.
- **Changes under `scripts/windows-vm/guest/` or `scripts/windows-pc/guest/`:** run `pc setup` to bring them to the PC. It restarts the runners when their files changed.
