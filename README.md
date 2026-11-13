<img src="assets/icon.svg" width="64" height="64" alt="">

# auto-clicker

An auto clicker with macro record and playback for KDE Plasma on Wayland. It lives in the system tray, and its hotkeys work inside fullscreen games. It also runs on niri and Hyprland ([setup](docs/guide/install.md#on-niri-and-hyprland)). Written in Rust to stay light while idle.

- **Click** fast, hold a button, repeat a key or hold one, at the cursor or a fixed point.
- **Record** what the mouse and keyboard do, and **play it back** with its timing, at any speed, as often as you like.
- **Nothing stays pressed**: stopping, quitting, locking the screen or sleep lets go of every button and key.
- **No network**: the program opens no connection, and installs only from this repository.

| Hotkey | Does |
| --- | --- |
| Ctrl+Shift+F6 | Start or stop clicking |
| Ctrl+Shift+F7 | Start recording, or stop and save it |
| Ctrl+Shift+F8 | Play the current recording, or stop it |

## Install

On CachyOS or Arch Linux, with Plasma, niri or Hyprland, one line at a time in a terminal:

```fish
sudo pacman -Syu
sudo pacman -S --needed base-devel git
git clone https://github.com/tobylo/auto-clicker.git ~/auto-clicker
cd ~/auto-clicker/packaging/arch && makepkg -sif
```

To record macros, and to have the hotkeys read from the keyboards rather than Plasma's shortcuts, the person who uses it also needs the `input` group. It lets every program of theirs read every keyboard and mouse, not just this one, so read [what the permissions mean](docs/guide/install.md#what-the-permissions-mean) first. On Plasma, clicking and Plasma's shortcuts work without it; on niri and Hyprland, give them the group, since the hotkeys are read from the keyboards there. To add it (`NAME` is their login name):

```fish
sudo usermod -aG input NAME
```

Restart the computer, then, logged in as that person:

```fish
systemctl --user enable --now auto-clicker
```

The icon appears in the tray. niri and Hyprland need a bar with a tray, and on Hyprland without uwsm a line in its config: see [On niri and Hyprland](docs/guide/install.md#on-niri-and-hyprland).

## Documentation

The [user guide](docs/guide/README.md) covers everything else:

- [Install, update and remove](docs/guide/install.md)
- [Using it](docs/guide/using.md): the tray, the hotkey, and when it stops by itself
- [Recording](docs/guide/recording.md) and its file format
- [Playing a recording](docs/guide/playback.md)
- [Settings window](docs/guide/settings-window.md)
- [Configuration](docs/guide/configuration.md): `config.toml` and key names
- [Routes: kernel or portal](docs/guide/routes.md): how clicks and hotkeys reach the desktop

What the program will do and why is in the [map](docs/planning/map.md); the work is on [the Project](https://github.com/users/tobylo/projects/4).

## Contributing

Bug reports and ideas are welcome as [issues](https://github.com/tobylo/auto-clicker/issues). The program installs only from this repository, and a pull request from outside is merged only by the maintainer, so it may wait a while or be turned down. Report a security problem privately, with "Report a vulnerability" under the repository's [Security tab](https://github.com/tobylo/auto-clicker/security), not in a public issue.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit for inclusion is licensed the same way, with no additional terms.
