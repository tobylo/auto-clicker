# Install, update and remove

These steps are for CachyOS (or Arch Linux) with Plasma. The computer builds the program itself from this repository; nothing else is downloaded except the Rust libraries it uses, from crates.io, at the exact versions in `Cargo.lock`. Each command is one line; paste it into a terminal (Konsole).

## Once per computer

Run these from an account that can use `sudo`.

1. Bring the system up to date, then restart the computer if the update included a new kernel:
   `sudo pacman -Syu`
2. Install the build tools:
   `sudo pacman -S --needed base-devel git`
3. Download this repository:
   `git clone https://github.com/tobylo/auto-clicker.git ~/auto-clicker`
4. Build and install the package. This takes a few minutes, installs Rust the first time, and asks for your password:
   `cd ~/auto-clicker/packaging/arch && makepkg -sif`

The package is called `tobylo-auto-clicker`. It installs the program, its tray icons, an "Auto Clicker" entry in the app menu, a background service, and a rule that lets the person at the screen use `/dev/uinput`.

## For each person who uses it

1. To record macros, let them read the keyboards and mice (replace `NAME` with their login name). Without this, the hotkeys go through Plasma's own shortcuts instead and recording is unavailable (see [Choosing the route](routes.md#choosing-the-route)):
   `sudo usermod -aG input NAME`
2. Restart the computer, so the new group and the `/dev/uinput` rule take effect.
3. Logged in as that person, start it now and at every login:
   `systemctl --user enable --now auto-clicker`

The icon then appears in the system tray. After **Quit**, start it again from "Auto Clicker" in the app menu, or log in again.

## What the permissions mean

- **`/dev/uinput`** lets a program create a virtual mouse and keyboard, which is how auto-clicker clicks. The rule lets programs of the person logged in at the screen open it, any of their programs, not just this one. When someone else takes over the screen, no new program of the first person can open it, but one that already has it open keeps it. auto-clicker itself stops clicking and lets go of everything when the screen locks or another user's session comes to the front.
- **The `input` group** lets a program read every keyboard and mouse directly, which recording needs, and which lets the hotkeys work without Plasma's shortcuts. auto-clicker only looks for its hotkeys, and records keys only while you record a macro. But the group applies to every program that person runs, not just this one: only add people whose programs you trust.
- The background service may use local Unix sockets only, so the program cannot reach the network even by mistake. The program itself opens no network connection.

## Updating

The program does not update itself. To install a newer version, from the account that installed it:

1. Fetch the new commits and list them:
   `cd ~/auto-clicker && git fetch && git log --oneline HEAD..origin/main`
2. Read what they change, all of it in one diff:
   `git diff HEAD origin/main`
   The program presses keys and reads keyboards: don't go on past a change you don't trust.
3. Bring them in, then build and install:
   `cd ~/auto-clicker/packaging/arch && git merge --ff-only origin/main && makepkg -sif`

Then restart the computer, or, for each person, `systemctl --user restart auto-clicker`. Always use `-f`: without it, makepkg would reinstall the package it built last time.

The build refuses to run if the downloaded copy has been changed by hand (`git status` shows changes), so what gets installed is always a version from this repository.

## If it does not start

`systemctl --user status auto-clicker` shows whether it runs, and `journalctl --user -b -u auto-clicker` shows its messages. Exit status 3 means another copy is already running. When it runs but cannot click or read hotkeys, the tray's tooltip says why and what to do.

## Uninstall

1. For each person who enabled it, logged in as them:
   `systemctl --user disable --now auto-clicker`
2. Remove the package:
   `sudo pacman -Rns tobylo-auto-clicker`
3. Take each person out of the `input` group, unless something else needs it, then restart the computer: programs already running keep the group until then.
   `sudo gpasswd -d NAME input`
4. Optionally, from the account that installed it, delete the downloaded copy:
   `rm -rf ~/auto-clicker`
   and Rust, if the build installed it and nothing else of yours needs it:
   `sudo pacman -Rns rust`
5. Optionally, as each person, delete their settings:
   `rm -rf ~/.config/auto-clicker`
