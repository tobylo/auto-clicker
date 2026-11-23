# M5: Run on Windows 11

- Milestone M5, scope gate G5. Investigation X03, slices WN01 to WN12 (issue numbers in [github-map.json](../github-map.json)).
- Maturity: mapped and settled 2026-11-23, before the spike. X03 may change any slice's open questions; a change to a decision below comes back to the owner.
- Diagram: [what stays, what Windows adds, and a hotkey click with a kill mid-hold](https://prlens.dev/c/IqawE6ShSVjC6_CUfjrqQg).

## Problem and outcome

The program runs only on Linux. Games run on Windows too, and the owner wants the same tool there. With M5 a user on Windows 11 gets the same program: a tray icon, Ctrl+Shift+F6 to click in the game, recording and playing routines, the settings window, and the countdown and badges on the screen.

## Scope

In scope: Windows 11 on x64, with feature parity with Linux v1 and O01: clicking, hotkeys, recording and playback with positions, the tray, the settings window, the on-screen display, English and Swedish. A test VM on the dev box. Install by building a reviewed commit on the Windows machine.

Out of scope: Windows 10; ARM64; macOS; games whose anti-cheat ignores injected input (the guide names the limitation, and the program never installs a driver or uses one, such as Interception); games running elevated, unless the user runs the program elevated too; an installer, code signing and an updater.

## Decisions

1. **One controller, both systems.** Core (controller, engine, recorder, config, recordings, the window's protocol, messages) is shared. Windows adds a backend, not a second design.
2. **Keys stay Linux key codes** in core, config and recordings. The Windows backend maps them to and from scan codes, and injects by scan code with the extended flag where a key needs it, since games reading raw input or DirectInput read scan codes.
3. **One crate, `backend-windows`, holds every Win32 call:** output, the watchdog's shared table, the message thread, hotkeys, raw input, screens and positions, the tray, single instance, session state and the on-screen display. Bindings come from `windows-sys`. `unsafe` stays in that backend, as the hard rule says. The Linux crates build only on Linux, through target-specific dependencies rather than scattered `cfg`.
4. **One Win32 message thread** owns every window, the tray icon, hotkeys, raw input and session notices. It sleeps in `GetMessage` while idle, sends the controller messages and changes no state itself. The controller and engine stay on tokio's single-thread runtime. Map D11 is amended to allow this second thread on Windows. At shutdown the thread is told to quit and joined with a deadline.
5. **Timing.** While a run is active, and only then, the daemon asks for 1 ms timer resolution (`timeBeginPeriod`) and opts out of power throttling for timers and speed. Windows 11 otherwise ignores a raised resolution for a program whose windows are all hidden, which a tray program's are. The 10 ms floor stays if X03 measures it holding; otherwise the floor becomes per system.
6. **Nothing stays pressed after the process dies.** Windows does not release what a dead program held. A watchdog (the daemon's own binary in a watchdog mode) shares a fixed-size table of held keys and buttons with the daemon, waits on the daemon's process, and releases whatever the table holds when the daemon ends. The daemon records a press in the table before sending it, so no click waits on I/O. Unwinding guards still cover panics, as on Linux. Killing the process tree or logging off ends both processes.
7. **Own events.** Every injected event carries the program's marker in its extra information. The recorder and any input reader drop marked events. Another program's injected input counts as the user's, as another uinput device does on Linux.
8. **Hotkeys** through `RegisterHotKey` with `MOD_NOREPEAT`, defaults unchanged (map D6), rebound in the settings window, which captures the keys from its own window events. A combination another program holds is refused by Windows: the tray names the hotkey and the reason, as map D2 does for a missing route.
9. **Recording** reads raw input (keys, buttons, relative motion), registered only while a recording runs, so the keyboard is read at no other time. Positions come from the pointer's position on each mouse move, and the layout from the monitors, in physical pixels with per-monitor DPI awareness. Low-level hooks are used only if X03 shows raw input cannot see what recording needs, because Windows silently removes a hook that answers slowly.
10. **Recordings name where their positions came from.** A recording made on Windows says so. Playback uses positions only from the system and layout they were made on, and relative motion otherwise, with the notice Linux gives for a changed layout.
11. **Files.** The same formats, under `%APPDATA%\auto-clicker` (config) and `%LOCALAPPDATA%\auto-clicker` (recordings, state, logs). Atomic writes use std only: rename to replace, a hard link then removing the temporary file where a save must not replace an existing file, and no-follow through the reparse-point flag. A sharing violation from an antivirus scanner or the indexer is transient, not "missing".
12. **Language** from the user's display language. Logs go to a file, since a program without a console has no output.
13. **The settings window** is the same Slint program on Slint's Windows backend. It talks to the daemon over its standard input and output pipes instead of a socket pair. The renderer is femtovg if it works on Windows, otherwise Slint's software renderer (WN10 decides).
14. **The tray** re-adds its icon when Explorer restarts (`TaskbarCreated`). Locking, switching user and sleep stop a run, as logind's facts do on Linux.
15. **Install:** the owner builds a reviewed commit on the Windows machine (`cargo build --locked --release`, MSVC toolchain). A PowerShell script, run with `-ExecutionPolicy Bypass`, copies the programs to `%LOCALAPPDATA%\Programs\auto-clicker`, registers start at login under the user's `Run` key, and uninstalls. The program is unsigned: SmartScreen asks before the first run, and Defender may flag a program that injects input; the guide says how to allow it.
16. **Tests.** A `windows-2025` CI job runs the three gates on the workspace. Device tests on Windows are `#[ignore]`d and run in the test VM (WN01) by the session, through a scheduled task in the VM's logged-in desktop, since a program started over SSH has no desktop to inject into. The PR names the VM's Windows build. What needs a real graphics card (a game at full speed, fullscreen behaviour, timing without a hypervisor) is measured on real hardware when the owner has some, and marked as VM-only until then.

## Invariants

- **N1, no second writer.** Everything Windows adds sends the controller messages; none of it changes state.
- **N2, idle costs nothing.** Idle on Windows is the message thread asleep in `GetMessage`, the runtime asleep, the watchdog asleep on the daemon's handle. No timer resolution is raised, no raw input is registered and nothing polls.
- **N3, every exit releases.** Stop, error, panic, lost output, session lock, sleep, shutdown and process death each release every key and button the program holds. Every stop test has a time bound well under the wait it interrupts.
- **N4, never reacts to itself.** No marked event is recorded, read as a hotkey or counted as user input.
- **N5, the keyboard is read only while recording.** Hotkeys need no keyboard reading on Windows. Raw keyboard input is registered only while a recording runs.

## X03: the spike, in the VM

X03 runs in the WN01 VM and ends with decision record 0006. It measures:

- SendInput clicks and keys reaching a test window and Minecraft Java (if it runs on the VM's OpenGL), counted as C02 counted them, at the 10 ms floor;
- interval accuracy at 10, 12 and 15 ms under tokio, with and without decision 5's settings, with all of the daemon's windows hidden (in a VM, so marked as an upper bound);
- what the marker and the raw-input device handle show for injected events, and whether a program can tell its own events from another injector's;
- whether raw input with `RIDEV_INPUTSINK` sees keys, buttons and relative motion while another window, including one reading raw input itself, has focus;
- whether `RegisterHotKey` fires while a game window has focus, and whether the game also gets the keys;
- that a key pressed with SendInput stays down after the process is killed, and that a watchdog's release clears it;
- a topmost, click-through, non-activating window over a borderless window, taking no focus and no input.

## Slices

About 1,500 added lines each. The clicker is usable after WN08.

| Code | Slice | Blocked by |
| --- | --- | --- |
| WN01 | Windows 11 test VM on the dev box: scripts to build it from the evaluation image, autologon, OpenSSH, the test runner task, rebuild every 90 days | — |
| X03 | The spike above, decision record 0006 | WN01 |
| WN02 | The workspace builds and its tests pass on Windows: target-specific dependencies, the CI job, Windows paths, language, log file | — |
| WN03 | Atomic files and file safety on Windows (decision 11) | WN02 |
| WN04 | Output backend: SendInput, scan codes, absolute moves, the marker, timing during runs | WN02, X03 |
| WN05 | The watchdog | WN04 |
| WN06 | The daemon on Windows: message thread, single instance, session lock and sleep, shutdown | WN02, X03 |
| WN07 | The tray on Windows | WN06 |
| WN08 | Hotkeys on Windows | WN04, WN06 |
| WN09 | Recording, screens and positions on Windows, recordings naming their source | WN04, WN06 |
| WN10 | The settings window and the fixed-point picker on Windows | WN06 |
| WN11 | The on-screen display on Windows | WN06 |
| WN12 | Install script, guide and README for Windows | WN03, WN05, WN07, WN08, WN09, WN10, WN11 |

## Open questions, owned by slices

- WN04: absolute moves over a virtual desktop whose origin may be negative; the scan-code table's source and its test.
- WN05: what the daemon does when the watchdog dies (start it again, or refuse runs with the reason), and the table's size.
- WN06: how the message thread and the runtime wake each other without polling.
- WN09: format version or `positions_from` value for Windows positions; how mouse moves and positions are paired.
- WN10: renderer; how the picker overlay covers each monitor at its own DPI.
- WN11: whether the display stays click-through over exclusive fullscreen; what shows when it cannot.
