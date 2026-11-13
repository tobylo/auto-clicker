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

## On niri and Hyprland

The package and the steps above are the same. Clicking, hotkeys, recording, playback and picking a point on the screen all work. On Hyprland, recordings also keep where the pointer was, once the service sees `HYPRLAND_INSTANCE_SIGNATURE` (below); on niri they keep mouse movement only, without where the pointer was (see [Recording](recording.md)), and the tooltip says so.

- **Starting the service.** The service starts with the desktop's `graphical-session.target`, and needs `WAYLAND_DISPLAY` in the systemd user environment.
  - **niri:** logging in to niri from the display manager runs `niri-session`, which does both, and clears the environment when the session ends. Nothing more is needed.
  - **Hyprland with uwsm** (the "Hyprland (uwsm-managed)" login entry) does both as well.
  - **Hyprland without uwsm** never starts `graphical-session.target`. Start the service from Hyprland's config instead, and clear what it imported when Hyprland exits, so a later session does not see a display that is gone. In the Lua config of Hyprland 0.57 and later (`~/.config/hypr/hyprland.lua`):

    ```lua
    hl.on("hyprland.start", function()
        hl.exec_cmd("systemctl --user import-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE && systemctl --user restart auto-clicker")
    end)
    hl.on("hyprland.shutdown", function()
        hl.exec_cmd("systemctl --user stop auto-clicker; systemctl --user unset-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE")
    end)
    ```

    With an older `hyprland.conf`, put the first command on an `exec-once =` line, and run the second when you log out.
- **The `input` group.** Give it to each person (above). The hotkeys come from Plasma's shortcuts only on Plasma: on niri and Hyprland they are read from the keyboards, which needs the group, as recording does.
- **The tray.** niri and Hyprland have no tray of their own: run a bar with a StatusNotifierItem tray, such as waybar with its `tray` module or ironbar. Without one, the program runs and its hotkeys work, and the icon appears once a bar starts.
- **Mouse acceleration.** Recordings on niri keep movement only, and so does movement on Hyprland while a game holds the pointer, so set the acceleration profile to flat: then the movement played back no longer depends on how fast it was recorded or played (see [Playing a recording](playback.md)); these settings apply to every mouse, yours and the program's virtual one alike. niri: `input { mouse { accel-profile "flat"; } }` in `~/.config/niri/config.kdl`. Hyprland: `hl.config({ input = { accel_profile = "flat" } })` in `hyprland.lua`, or `accel_profile = flat` under `input` in `hyprland.conf`.
- **Picking a point on niri.** At a fractional scale where a screen's size in pixels divided by the scale is not whole, a picked point near the screen's right or bottom edge can land up to 1 pixel off: niri rounds that size one way for the program's virtual device and the other way for the screen it reports.
- **A virtual device bound to one screen** in Hyprland's config is not supported: picking and playback assume the program's devices span every screen, as they do by default.

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

The build refuses to run if the downloaded copy has changes that are not committed (`git status` shows them), so it always builds a commit. It builds whatever commit is checked out, though: the steps above keep that to commits from this repository.

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
6. Optionally, as each person, delete their recordings, which can hold the keys they typed while recording, and the program's remembered state (the current recording and Plasma's portal token):
   `rm -rf ~/.local/share/auto-clicker ~/.local/state/auto-clicker`
