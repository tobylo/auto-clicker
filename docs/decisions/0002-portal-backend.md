# 0002: The portal backend on Plasma 6.8

Status: accepted (X02, #8, 2026-11-10). Plans M4: P01 (#19), P02 (#20), P03 (#21), P04 (#22).

## Decision

| Area | Result | What M4 builds |
| --- | --- | --- |
| Output (RemoteDesktop + libei) | **Works** | `InputBackend` on `ashpd` + `reis`, with a restore token so the prompt shows once |
| Hotkeys (GlobalShortcuts) | **Works, with limits** | Bind on every start; rebinding goes through KDE's shortcut settings, not the app; the program's own key output triggers its hotkeys |
| Recording (InputCapture) | **Not usable for recording** | Record is disabled on the portal backend (P03's default) |
| Detection | **Works** | One result per capability (output, hotkeys, recording), not one backend for everything |

Every portal call needs an **app id**. A host program gets one only when a `.desktop` file for that id is installed and the program either runs in a systemd unit named `app-<id>.service` or calls `org.freedesktop.host.portal.Registry.Register`. The package must install the `.desktop` file, and the user service must carry that name (or the daemon registers).

## Setup

- Laptop `asus-m16` (i9-13900H), CachyOS, kernel 7.3.3-1-cachyos. Plasma 6.8.2 on Wayland: KWin 6.8.2, plasma-workspace 6.8.2, xdg-desktop-portal 1.22.1, xdg-desktop-portal-kde 6.8.2, libei 1.6.0. Two outputs during the runs: eDP-1 2048x1280 logical at scale 1.25, and DP-1 3440x1440 at scale 1.
- Portal versions as reported on D-Bus: RemoteDesktop 2 (`AvailableDeviceTypes` 7), GlobalShortcuts 2, InputCapture 2 (`SupportedCapabilities` 7).
- The spike: [spikes/portal-backend](../../spikes/portal-backend/README.md), on `ashpd` 0.14.0 (zbus 6, tokio) and `reis` 0.7.1 (tokio), on tokio's single-thread runtime (D11). It was run from the dev box over SSH, as a systemd user service named `app-io.github.tobylo.AutoClickerX02.service` (`run-unit.sh`), with a spike-only id so that its grants never pre-authorise the real program.
- What a window received was read from the probe window's `WAYLAND_DEBUG=1` log (`run-probe.sh`, `wayland_events.py`): raw `wl_pointer.button` and `wl_keyboard.key` events with the compositor's millisecond times. These include releases the compositor sends on its own.

## App identity

| Launch | Result |
| --- | --- |
| Plain process (from a terminal or SSH) | `CreateSession` fails: `org.freedesktop.portal.Error.NotAllowed: An app id is required` (GlobalShortcuts) |
| `--app-id` (Registry.Register) without a `.desktop` file | `Could not register app ID: App info not found for 'io.github.tobylo.AutoClickerX02'` |
| systemd unit `app-<id>.service` without a `.desktop` file | Same as a plain process |
| `.desktop` file installed, then Register | Works |
| `.desktop` file installed, unit `app-<id>.service`, no Register | Works |

Source: xdg-desktop-portal 1.22.1 `src/xdp-app-info-host.c:182-191` takes the id from the systemd user unit only when the unit starts with `app-` and `g_desktop_app_info_new("<id>.desktop")` finds the file. `src/global-shortcuts.c:254-260` rejects an empty id ("shortcuts really need to be scoped to a specific app"). RemoteDesktop accepts an empty id, but its permission would then be stored under the empty id, shared with every unidentified program.

KDE shows the `.desktop` file's `Name` in the permission dialogs and in System Settings ("Auto Clicker X02 spike").

## Output: works

```sh
./run-unit.sh output rd.token grant      # the prompt; Allow (owner)
./run-unit.sh output rd.token park       # then, with the probe window open:
./run-probe.sh logs/demo ./run-unit.sh output rd.token demo
```

- **Permission.** `SelectDevices` with keyboard | pointer, `persist_mode` 2 (until revoked), then `Start`. The first `Start` showed KDE's dialog and answered after the owner clicked Allow (6.2 s). Every later start, each in a new process, answered in 7 to 8 ms with no dialog, from the saved token. KDE returned **the same token** each time, so a lost token write does not lock the program out; the program still saves whatever `Start` returns, as the spec asks.
- **Devices.** After the handshake and a seat bind, KWin announces three resumed virtual devices: `eis absolute device` (pointer absolute, button, scroll; one region per output, in logical pixels: `0,0 2048x1280 scale 1.25 eDP-1` and `2048,0 3440x1440 scale 1 DP-1`), `eis pointer` (relative pointer, button, scroll; no regions) and `eis keyboard`.
- **Each method of `InputBackend`:**

| Method | libei request | Result |
| --- | --- | --- |
| `button` | `ei_button.button(BTN_*, press/released)` + frame | Left, right and middle arrived as 0x110/0x111/0x112 down and up. Side and extra use the same codes |
| `key` | `ei_keyboard.key(KEY_*, press/released)` + frame | `KEY_A` arrived as key 30; evdev codes pass through unchanged. KWin sends the key repeats itself while a key is held (`wl_keyboard.key` state 2, 60 in 3 s) |
| `move_relative` | `ei_pointer.motion_relative(dx, dy)` + frame | +100,+50 moved the pointer exactly 100,50 logical pixels, with no acceleration. Turned the view in fullscreen Minecraft |
| `move_absolute` | `ei_pointer_absolute.motion_absolute(x, y)` + frame | Coordinates are logical pixels on the layout, inside a region: (1024, 640) and (512, 320) arrived exactly. The normalised 0..=u16::MAX of `AbsolutePosition` has to be mapped onto the union of the regions; the kernel backend's stretch over all outputs is the same idea |
| `scroll` | `ei_scroll.scroll_discrete(0, ±120)` + frame | One notch per 120; positive is down (GTK reported +1 then -1). The trait's "positive scrolls up" needs a sign flip |

- **Click rate.** `rate 1000 10`, `rate 1000 5` and `rate 1000 2`: the window received 1000 of 1000 presses and releases at each interval, with no failed writes. Periods by the compositor's millisecond clock: 9/10/11, 4/5/6 and 1/2/3 ms (min/median/max). The portal route matches uinput (C02); the 10 ms floor stands for both.
- **Fullscreen game.** In fullscreen Minecraft (native Wayland, pointer locked) ten relative steps turned the view, ten slow right clicks and 100 left clicks at 10 ms registered (owner, by eye).

### Release: the compositor releases held input on every exit

`hold button|key HOW`, run for each HOW; times are the compositor's.

| End | Button released | Key released | Afterwards |
| --- | --- | --- | --- |
| SIGKILL | at once | at once | — |
| Process exits without releasing | 1 ms after exit | 1 ms after exit | — |
| Portal session closed (`Session.Close`) | within 2 ms | within 2 ms | Devices removed, then `ei_connection.disconnected` (reason `Disconnected`); a write fails as "no device" |
| EIS socket disconnected, portal session kept | at once | at once | Devices removed; the stream ends |
| `stop_emulating` | 950 ms later | 950 ms later | Connection stays; the next write works after `start_emulating` |

This matches KWin's code (master, read 2026-11-10): `src/plugins/eis/eisdevice.cpp:36-41` (the device's destructor calls `releasePressedAndTouches`), `:43-57` (releases every pressed button and key), `:31-33` and `:72-79` (a 1 s single-shot timer after stop-emulating). The `Input` guard's own `release_all` stays the first line, since it reports failures and releases at once, but on this backend a crash cannot leave a button down, just as on uinput.

### The shape of the backend

reis requests are synchronous: each is buffered, and `flush` writes it to the non-blocking socket. The `InputBackend` methods can therefore stay synchronous. What needs the runtime is the event stream. The compositor pings the client, announces and removes devices, and pauses and resumes them, and nothing answers unless a task polls the stream. The spike runs one local task that drains it, keeps the device table current, and **flushes after every event**. Without that flush, the seat bind and the ping replies sit in the buffer and no device ever appears; this was the first run's failure.

Writes check the device table: a write after the session ended returned "no resumed device", which the backend maps to a permanent `InputError` (session lost). Pause and resume were not seen in these runs; a paused device stops emulating, and KWin's stop-emulating timer would then release what it held.

## Hotkeys: works, with limits

```sh
./run-unit.sh shortcuts bind 200       # dialog; accept (owner)
./run-unit.sh shortcuts list 150       # new session, no bind
./run-unit.sh shortcuts bind 150       # new session, bind again
./run-unit.sh shortcuts rebind 5       # ask for Ctrl+Shift+F6/F7/F8
./run-unit.sh shortcuts configure 240  # ConfigureShortcuts
```

- **First bind.** KDE showed a dialog listing the three shortcuts, flagged Ctrl+F7 as already in use, and the owner accepted. `BindShortcuts` answered with all three, Ctrl+F7 included, and wrote them under `[io.github.tobylo.AutoClickerX02]` in `~/.config/kglobalshortcutsrc`.
- **Activations.** Ctrl+F6 and Ctrl+F8 arrived as `Activated` and `Deactivated` (100 to 150 ms apart, key down to key up), on the desktop and **in fullscreen Minecraft**, and the game did not see them. Every signal carried timestamp 0.
- **Conflict.** Ctrl+F7 is KWin's own default for "Toggle Present Windows (Window class)". Pressing it opened Present Windows and never reached the program, although the portal kept reporting "Ctrl+F7" as the trigger. The program cannot tell from the portal that a binding is dead. **D6's default Ctrl+F7 collides with a stock Plasma shortcut**; Ctrl+F9 and Ctrl+F10 are taken too.
- **Persistence.** A new session in a new process listed the three shortcuts without binding, but **received no activations** until it called `BindShortcuts` again. That second bind answered in under a second **with no dialog**. The daemon binds on every start; the prompt appears once.
- **Rebinding.** `BindShortcuts` with other preferred triggers in a later session changed nothing: the answer and the config still held the first triggers. The app cannot change a binding. `ConfigureShortcuts` (version 2) opened System Settings on the program's shortcuts. The owner changed recording to Ctrl+Shift+F7 there, the session received `ShortcutsChanged` with the new trigger, and Ctrl+Shift+F7 then activated it.
- **The program's own input.** Ctrl+F6 sent through libei activated the `clicking` shortcut. An activation carries no device, so the portal backend cannot tell its own key output from the user's by device identity (AGENTS.md: "The program never reacts to itself").

## Recording: InputCapture takes input from the game

```sh
./run-probe.sh logs/capture ./run-unit.sh --deadline 200 capture 150   # dialog; Allow; push against the top edge (owner)
```

- `CreateSession` showed KDE's dialog; the owner allowed it. `GetZones` returned one zone per output, and barriers along both top edges were accepted.
- Pushing the pointer against the top edge activated capture (barrier 2, DP-1) six times. Each time the spike handed input back after 3 s.
- During the six captures the focused probe window received **0** pointer events; outside them, in the same run, 4442. It received no `leave` either: input simply stopped reaching it. That is what the portal spec describes, and what rules InputCapture out for recording a game: the recording would take the input the game needs.
- A second run (one capture, `capture 120`) counted what the capturing program received: 77 motion events and a button press and release, while the focused window again received 0 pointer events during the capture and 1025 outside it. Capture moves the input from the window to the capturing program.
- Each run's `CreateSession` showed the permission dialog again: InputCapture version 1 sessions keep no grant. Version 2's `Start` takes a restore token, but persistence would not change the result above.
- No portal or KWin interface observes input while leaving it with the focused window. KWin's position has long been to offer no global key logging. Recording therefore stays on the kernel backend (D2 unchanged).
- KDE bug [523515](https://bugs.kde.org/show_bug.cgi?id=523515), "InputCapture session not torn down when the portal frontend crashes -> all keyboard/pointer input stays diverted": reported against 6.6.4, fixed in 6.6.7 ([MR 607](https://invent.kde.org/plasma/xdg-desktop-portal-kde/-/merge_requests/607)). 6.8.2 carries the fix. It shows the failure mode: a stuck capture freezes the desktop's input.
- The "6.8 dialog issue": not seen. The dialog appeared at once in all three runs and allowed the session. The report found in research (a missing `InputCaptureDialog` QML type, so no dialog at all) was against 6.7.4 and fixed in xdg-desktop-portal-kde 6.7.5; it carries no KDE bug number.

## Detection: works, per capability

```sh
./target/release/portal-backend-spike detect
```

On the laptop:

```text
uinput: writable
evdev: 0 of 32 event nodes readable (32 denied, 0 other errors)
RemoteDesktop: Ok((2, DeviceType(7)))
GlobalShortcuts: Ok(2)
InputCapture: Ok(2)
output: kernel (uinput)
hotkeys: portal (GlobalShortcuts)
recording: none
```

The owner's own laptop is the mixed case: uinput through the uaccess rule, no `input` group. A single "kernel or portal" choice gets it wrong either way: kernel loses hotkeys, portal costs a prompt that uinput does not need. Detection therefore reports each capability. Whether the program mixes backends (uinput output with portal hotkeys) is P04's to decide.

- Open the device node and keep the handle (the checked thing is the used thing), and tell `EACCES` from `ENOENT` and other errors.
- The portal side: the frontend's `version` properties (RemoteDesktop ≥ 2 for `ConnectToEIS`; ConfigureShortcuts needs GlobalShortcuts ≥ 2) and `AvailableDeviceTypes` containing keyboard and pointer. A missing app id is only found by calling: `CreateSession` fails with `NotAllowed`.
- A denied prompt answers `Start` with response 1, which ashpd reports as "Portal request was cancelled". That is distinct from `NotAllowed` (no app id) and from a missing interface or version. The deny left the existing grant in place: the saved token still restored in 7 ms.

## Restarts

- Daemon restart: every run above was a new process; RemoteDesktop restored in 7 to 8 ms with no dialog, and GlobalShortcuts bound again with no dialog.
- Reboot (owner, 2026-11-10 01:23): after logging back in, RemoteDesktop restored from the saved token in 11 ms with no dialog, and GlobalShortcuts bound again with no dialog, keeping the trigger the owner had changed in System Settings.
- The grant lives in the permission store (`remote-desktop` table, keyed by the token, holding the app id and the granted devices). The shortcuts live in `~/.config/kglobalshortcutsrc` under the app id.

## Dependencies for M4

`ashpd` 0.14.0 and `reis` 0.7.1 are pure Rust with no system libraries. reis is `#![forbid(unsafe_code)]`; ashpd's `unsafe` sits only in its GTK and Wayland window-identifier code, which these features leave out. The spike's tree is 56 crates; the release binary is 2.9 MB stripped, and it builds from clean in 25 s on the laptop. ashpd 0.14 uses zbus 6, while `ksni` 0.3.7 (D1, the tray) uses zbus 5, so a daemon with both builds two zbus majors; P01 should check whether a newer ksni or an older ashpd lines them up. ashpd 0.14.0's `remote_desktop` feature does not build without `screencast` (an upstream bug; enable both).

## Upstream reports for the owner

Neither is reported yet (checked bugs.kde.org and the ashpd issues and pull requests on 2026-11-10). The owner files them.

**1. xdg-desktop-portal-kde: GlobalShortcuts reports a trigger that never activates.**
- Reproducer: with stock KWin shortcuts (Ctrl+F7 is "Toggle Present Windows (Window class)"), call `BindShortcuts` with one shortcut whose `preferred_trigger` is `CTRL+F7`, and accept the dialog, which flags the conflict. Then press Ctrl+F7.
- Observed: the `BindShortcuts` response and every later `ListShortcuts` give `trigger_description` "Ctrl+F7", and kglobalshortcutsrc stores `Ctrl+F7` for the app. Pressing it opens Present Windows; the app gets no `Activated`.
- Expected: a shortcut that cannot activate is reported without a trigger (or the conflict is resolved), so the app can tell the user.
- Source: `src/globalshortcuts.cpp:494-506` (v6.8.2) builds `trigger_description` from `action->shortcuts()` without checking `KGlobalAccel::isGlobalShortcutAvailable`, which the dialog does use (`:62`, `:142`).
- Where it hurts: an app's hotkey is dead while the app and System Settings both show it as bound. Here, D6's default record hotkey.

**2. ashpd 0.14.0: the `remote_desktop` feature does not build alone.**
- Reproducer: `ashpd = { version = "0.14.0", default-features = false, features = ["tokio", "remote_desktop"] }`, then `cargo build`.
- Observed: `error[E0432]: unresolved import super::screencast` at `src/desktop/remote_desktop.rs:99` (`screencast::Stream`). Still so on master (`client/src/desktop/remote_desktop.rs:99`; `remote_desktop = ["dep:serde_repr"]` in `client/Cargo.toml`).
- Expected: `remote_desktop` enables `screencast`, or the `Stream` import is gated.
- Where it hurts: a build-time annoyance; enabling `screencast` works around it.

Noted but not reported:
- Activation timestamps are 0. xdg-desktop-portal-kde passes kglobalaccel's timestamp through (`src/globalshortcuts.cpp:383-398`), so the zero comes from kglobalaccel or KWin; the program does not need the field.
- KWin drops an EIS client's devices without emitting `deviceRemoved` on client disconnect (`src/plugins/eis/eiscontext.cpp:198-205`, unlike the device-closed path at `:231-247`). This is from reading the code only. Held input was still released (see the release table), so there is no observed effect.
