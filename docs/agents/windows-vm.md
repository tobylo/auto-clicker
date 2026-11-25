# The Windows test VM

Sessions run Windows device tests themselves, in a Windows 11 Enterprise evaluation VM on the dev box (WN01, brief decision 17). `scripts/windows-vm/vm` builds it, starts it, runs programs in its desktop, captures its screen and stops it. Nothing here needs the owner's hands.

## What it is

- **Windows:** Windows 11 Enterprise evaluation, 26H2 (build 26300 and its updates), from Microsoft's own download. The build refuses any other file: the SHA-256 that Microsoft publishes is pinned in `lib/common.sh`.
- **QEMU/KVM:** a q35 machine with UEFI, a software TPM (swtpm), 8 cores, 12 GB of RAM, a 128 GB disk and a 1920×1080 screen. It has no window on the dev box and never takes the dev box's mouse or keyboard.
- **Accounts:** one local user, `tester`, logs in automatically at boot. It is an administrator with UAC on.
  - SSH accepts only the key made at build time, and it lands elevated.
  - The runner's normal queue runs unelevated, as the daemon must.
  - The password is made per build and kept in the state directory. Windows keeps it readable in its registry for automatic logon, so `tester` is used for nothing else.
- **Files:** everything lives in `~/.local/share/auto-clicker-vm/`, outside the repository:
  - `cache/`: the ISO and the OpenSSH installer;
  - `vm/`: the clean build, the key, the password and `info.json`;
  - `vm/run/`: the running copy and its logs.
- **Toolchain:** Visual Studio Build Tools (the MSVC linker and the Windows SDK) and Rust stable with clippy and rustfmt, plus the OpenGL compatibility pack from the Store.

## Network

The VM reaches nothing by itself. QEMU's user networking runs with `restrict=on`, so the VM reaches no address on the LAN, none of the dev box's own services, and nothing on the internet. Two openings are left:

- **SSH in:** QEMU forwards `127.0.0.1:52222` on the dev box (`WN01_SSH_PORT` changes it) to the VM's port 22. It listens on loopback only, so other machines cannot reach it.
- **The proxy out:** connections to `10.0.2.100:3128` reach `proxy.py`. QEMU starts a copy for each connection, so it listens on nothing.
  - It passes HTTPS (`CONNECT` to port 443) and plain HTTP (port 80) to listed host names only.
  - It refuses address literals, and any name that resolves only to private, loopback or link-local addresses.
  - While a build runs, it allows `allow-run.txt` and `allow-build.txt`: Windows Update, activation, Visual Studio Build Tools, winget's Store source, Rust and crates.io.
  - In normal runs, it allows `allow-run.txt` only: Rust and crates.io. Windows Update cannot reach its servers between builds, so it cannot restart the VM in the middle of a test.
- **One exception, during the build only:** the Microsoft Store's download service ignores the proxy, so for the OpenGL compatibility pack the build hot-plugs a second network card on unrestricted user networking, which reaches the internet and the LAN. Once the pack is installed it removes the card, and the build fails unless QEMU confirms the card is gone within a minute. A built VM never has that card.

Every request, passed or refused, is logged to `proxy.log` in the machine's directory: `vm/run/` for a running VM, the build directory during a build. When a build or a `cargo` fetch fails on a refused host, that log names the host. A host goes into the list it belongs to in a PR, never by hand on the dev box.

## Commands

All of them print to stderr as they go, and every wait in them has a deadline.

| Command | What it does |
| --- | --- |
| `vm build` | Builds the VM from nothing: download and checks, unattended setup, SSH, the runners, activation, Build Tools, Rust, the OpenGL pack, Windows Update until none are left. About 30 minutes on the dev box, longer when Windows Update has much to install. |
| `vm start` | Boots the VM and returns once SSH and both runners answer (deadline 15 minutes). |
| `vm run [--elevated] [--timeout S] [--workdir DIR] -- COMMAND` | Runs `COMMAND` through `cmd.exe /c` in tester's desktop, so pipes, `&&` and redirection work. It prints the command's stdout and stderr and exits with its exit code. |
| `vm screen [FILE.png]` | Captures the screen to a PNG on the dev box. |
| `vm key KEYS...` | Presses keys in the VM, by QEMU names: `ret`, `esc`, `ctrl-alt-delete`, `meta_l-r`. |
| `vm push PATH [DIR]` | Copies a local file or folder into `DIR` in the VM (default `C:\work`), as a tar stream over SSH. `scp` and `sftp` do not work: Windows' OpenSSH server ends its SFTP subsystem at once. |
| `vm ssh [COMMAND]` | Runs PowerShell over SSH as tester, elevated, with no desktop. Use it for files, builds and inspection. |
| `vm selftest` | Checks the running VM: an injected click reaches a window, the screen is captured, SSH listens on loopback only, and the VM reaches nothing it should not. |
| `vm status` | Shows the Windows build, days until the evaluation expires, and whether the VM is running. |
| `vm stop` | Shuts Windows down (3 minutes, then the VM is ended) and stops QEMU and the TPM. |
| `vm reset` | Discards every change made since the build. The next start boots the clean build. |

`vm run` details:
- The default `--timeout` is 120 seconds and the maximum is 3600. At the deadline, the command's whole process tree is ended: it runs in its own Windows job object, and so does everything it starts.
- Exit code 124 means the deadline hit, 125 that the runner could not start the command, and 126 that the runner did not answer.
- `--elevated` runs the command through the second runner, which has administrator rights in the same desktop. Use it for a window that is meant to be elevated, as in a UIPI test.

## Running a test

A program started over SSH has no desktop: it cannot inject input or own a visible window. Use `vm ssh` for anything without a desktop, and `vm run` for anything that clicks, types or shows a window.

```sh
scripts/windows-vm/vm start
# Copy the tree at HEAD into the VM and build it there.
git archive --format=tar --prefix=src/ HEAD | scripts/windows-vm/vm ssh "cmd /c 'tar -x -f - -C C:\work'"
scripts/windows-vm/vm ssh 'cd C:\work\src; cargo build --locked'
# Device tests run in the desktop, one at a time.
scripts/windows-vm/vm run --timeout 600 --workdir 'C:\work\src' -- cargo test --locked -- --ignored --test-threads=1
scripts/windows-vm/vm screen .claude/local/after-test.png
scripts/windows-vm/vm stop
```

- One session uses the VM at a time. `vm run`, `start`, `stop`, `reset`, `build` and `selftest` take a lock: a second session waits, and gives up with a message once its deadline passes. Stop the VM when your runs are done. It never starts on its own.
- The PR names the VM's Windows build, as `vm status` prints it.
- A measurement that needs a real graphics card (a game at full speed, fullscreen behaviour, timing without a hypervisor) is marked VM-only.

## Resetting and rebuilding

- **Reset:** `vm stop && vm reset` puts the VM back to the clean build in seconds. Reset before a measurement that must not depend on earlier runs, and after a test that changed Windows settings.
- **Rebuild:** the evaluation runs for 90 days from the build. `vm status` and `vm start` warn once fewer than 14 days are left. Then run `vm stop` and `vm build`.
  - The new VM is built beside the old one and replaces it only once the build has succeeded.
  - A failed or interrupted build removes its half-made files, and the old VM stays as it was.
  - A build killed outright leaves a directory that the next build removes.
  - Run `vm selftest` on the new VM before using it.
- **Changes under `scripts/windows-vm/guest/`:** the runner, setup and provisioning scripts are copied into the VM at build time, so a change to them takes a rebuild to apply. `selftest` copies its two check scripts in each time.
- **A new ISO:** when Microsoft replaces the evaluation image, the build stops on the SHA-256 mismatch. Update `ISO_URL`, `ISO_SHA256` and `ISO_FILE` in `lib/common.sh` from Microsoft's verification page (https://aka.ms/VerifyWindowsISO) in a PR. Never take the image from a mirror.

## When something fails

- **Look at the screen first:** run `vm screen`, or, during a build, send `screendump <file> -f png` to the build directory's `monitor.sock` with `socat`.
- **Logs on the dev box:** `qemu.log`, `swtpm.log` and `proxy.log`, in `vm/run/` or in the build directory.
- **A failed build** keeps its logs and its last screen in `last-failed-build/` in the state directory, and removes everything else it made.
- **Logs in the VM:**
  - `C:\wn01-setup.log`: first-logon setup;
  - `C:\wn01\queue\<queue>\runner.log`: runner errors;
  - `C:\wn01\windows-update.json`: the last Windows Update pass.
