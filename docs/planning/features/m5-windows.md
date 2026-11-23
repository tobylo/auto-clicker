# M5: Run on Windows 11

- Milestone M5, scope gate G5 (#107). Investigation X03 (#109), slices WN01 (#108) and WN02 to WN12 (#110 to #120); [github-map.json](../github-map.json) maps the codes.
- On Windows this brief replaces map D1's tray, D7's paths, file calls and input reader, and D12's layer-shell surfaces; everything else in the map holds on both systems.
- Maturity: mapped and settled 2026-11-23, before the spike. X03 may change any slice's open questions; a change to a decision below comes back to the owner.
- Diagram: [what stays, what Windows adds, and a hotkey click with a kill mid-hold](https://prlens.dev/c/IqawE6ShSVjC6_CUfjrqQg).

## Problem and outcome

The program runs only on Linux. Games run on Windows too, and the owner wants the same tool there. With M5 a user on Windows 11 gets the same program: a tray icon, Ctrl+Shift+F6 to click in the game, recording and playing routines, the settings window, and the countdown and badges on the screen.

## Scope

In scope: Windows 11 on x64, with feature parity with Linux v1 and O01: clicking, hotkeys, recording and playback with positions, the tray, the settings window, the on-screen display, English and Swedish. A test VM on the dev box. Install by building a reviewed commit on the Windows machine.

Out of scope: Windows 10; ARM64; macOS; games whose anti-cheat ignores injected input (the guide names the limitation, and the program never installs a driver or uses one, such as Interception); running elevated, and so games running elevated: an elevated daemon would read config, recordings and its own binary from places any unelevated program of the user can write, and become a way past UIPI, so the daemon refuses to start elevated and says why; an installer, code signing and an updater.

## Decisions

1. **One controller, both systems.** Core (controller, engine, recorder, config, recordings, the window's protocol, messages) is shared. Windows adds a backend, not a second design.
2. **Keys stay Linux key codes** in core, config and recordings. The Windows backend maps them to and from scan codes, and injects by scan code with the extended flag where a key needs it, since games reading raw input or DirectInput read scan codes.
3. **One crate, `backend-windows`, holds every Win32 call:** output, the watchdog's shared table, the message thread, hotkeys, raw input, screens and positions, the tray, single instance, session state and the on-screen display. Bindings come from `windows-sys`. `unsafe` stays in that backend, as the hard rule says. The Linux crates build only on Linux, through target-specific dependencies rather than scattered `cfg`. Core's own Unix file code (`fs.rs`, config reads, the recordings library) is ported, not hidden: core builds and tests on both systems. Windows-only dependencies are checked by the same advisory and licence gate as the rest.
4. **One Win32 message thread** owns every window, the tray icon, hotkeys, raw input and session notices. It sleeps in `GetMessage` while idle, sends the controller messages and changes no state itself. The controller and engine stay on tokio's single-thread runtime. Map D11 is amended to allow this second thread on Windows. At shutdown the thread is told to quit and joined with a deadline.
5. **Timing.** While a run is active, and only then, the daemon asks for 1 ms timer resolution (`timeBeginPeriod`) and opts out of power throttling for timers and speed. Windows 11 otherwise ignores a raised resolution for a program whose windows are all hidden, which a tray program's are. The 10 ms floor stays if X03 measures it holding; otherwise the floor becomes per system.
6. **Nothing stays pressed after the process dies.** Windows does not release what a dead program held. A watchdog, a small program of its own (`auto-clicker-watchdog.exe`, so a kill by the daemon's name does not take it), is what starts at login: it starts the daemon as its child, shares a fixed-size table of held keys and buttons with it, waits on the daemon's process, and releases whatever the table holds when the daemon ends, then exits. Being the parent, it survives "End process tree" on the daemon. The daemon sends no press before the watchdog is attached, records a press in the table before sending it and clears it only after the release is sent, so no click waits on I/O; a full table refuses the press. The watchdog reads the table as untrusted: bounded, validated, releases only. Unwinding guards still cover panics, as on Linux. What remains: killing the watchdog and the daemon together (the watchdog's own tree) and logging off; the guide names both.
7. **A release Windows blocks is sent again.** UIPI drops injected input while an elevated window has focus, and the secure desktop (lock, UAC) takes all input, without an error. A held key whose release may not have arrived stays in the table and is released again once the user's desktop has input again. X03 measures both cases.
8. **Own events.** Every injected event carries the program's marker in its extra information. The recorder and any input reader drop marked events. Another program's injected input counts as the user's, as another uinput device does on Linux.
9. **Hotkeys** through `RegisterHotKey` with `MOD_NOREPEAT`, defaults unchanged (map D6), rebound in the settings window, which captures the keys from its own window events while the daemon's hotkeys are unregistered. A hotkey message carries no marker, so, as on the portal route (P02), the program never presses a hotkey's key: a run that would is refused, judged by the registered bindings. A hotkey message says nothing about the keys coming up, and the controller starts a run only once the hotkey's modifiers are up (so the game never gets Ctrl+Shift+click): from a hotkey's activation until its modifiers are up, or the existing 30 s cancel, the backend registers raw keyboard input and reads only whether the modifiers are down, logging nothing. A combination another program holds is refused by Windows: the tray names the hotkey and the reason, as map D2 does for a missing route, and a save registers the new set before releasing the old one, so a refused combination refuses the whole save.
10. **Recording** reads raw input (keys, buttons, relative motion), registered only while a recording runs (and, for the modifiers alone, decision 9's wait). Positions come from the pointer's position on each mouse move, and the layout from the monitors, in physical pixels with per-monitor DPI awareness. Low-level hooks are used only if X03 shows raw input cannot see what recording needs, because Windows silently removes a hook that answers slowly.
11. **Recordings name where their positions came from.** A recording made on Windows says so. Playback uses positions only from the system and layout they were made on, and relative motion otherwise, with the notice Linux gives for a changed layout.
12. **Files.** The same formats, under `%APPDATA%\auto-clicker` (config) and `%LOCALAPPDATA%\auto-clicker` (recordings, state, logs). Atomic writes keep Linux's guarantees, the checked directory being the one written to included: replace by rename, create-new by a hard link then removing the temporary file, no-follow through the reparse-point flag. Linux reaches every name through the checked directory's open handle; std has no such call on Windows, so there the directory's path is resolved once into one through no junction, the directory itself is refused if it is a reparse point, and it is held open without delete sharing, so nobody can rename or delete it, or its parents, while it is held, and that resolved path keeps naming it (WN02, settled 2026-11-24). Core stays free of `unsafe` and uses std alone, as cap-std's own Windows code does the same. Each file is flushed (`sync_all`) before the rename or link. Windows has no directory fsync, so that step of AGENTS.md's rule does not apply there. A sharing violation from an antivirus scanner or the indexer is transient, not "missing".
13. **Language** from the user's display language. Logs go to a file, since a program without a console has no output.
14. **The settings window** is the same Slint program on Slint's Windows backend. It talks to the daemon over its standard input and output pipes instead of a socket pair. The renderer is femtovg if it works on Windows, otherwise Slint's software renderer (WN10 decides).
15. **The tray** re-adds its icon when Explorer restarts (`TaskbarCreated`). Locking, switching user and sleep stop a run, as logind's facts do on Linux.
16. **Install:** the owner builds a reviewed commit on the Windows machine (`cargo build --locked --release`, MSVC toolchain). A PowerShell script, run with `-ExecutionPolicy Bypass`, copies the programs to `%LOCALAPPDATA%\Programs\auto-clicker`, registers the watchdog (which starts the daemon) to start at login under the user's `Run` key, upgrades a running install by asking the daemon to quit first, and uninstalls. The program is unsigned: SmartScreen asks before the first run, and Defender may flag a program that injects input; the guide says how to allow it.
17. **Tests.** A `windows-2025` CI job runs the gates of `scripts/gates.sh` on the workspace, and cargo-deny checks the Windows target too. Before a commit, Windows-only code is checked on the dev box with clippy for the Windows target, and its tests run in the VM. Device tests on Windows are `#[ignore]`d and run in the test VM (WN01) by the session, through a scheduled task in the VM's logged-in desktop, since a program started over SSH has no desktop to inject into (WN01). The PR names the VM's Windows build. What needs a real graphics card (a game at full speed, fullscreen behaviour, timing without a hypervisor) is measured on real hardware when the owner has some, and marked as VM-only until then.

## Invariants

- **N1, no second writer.** Everything Windows adds sends the controller messages; none of it changes state.
- **N2, idle costs nothing.** Idle on Windows is the message thread asleep in `GetMessage`, the runtime asleep, the watchdog asleep on the daemon's handle. No timer resolution is raised, no raw input is registered and nothing polls.
- **N3, every exit releases.** Stop, error, panic, lost output, session lock, sleep, shutdown and the daemon's death, its process tree included, each release every key and button the program holds; a release Windows blocked is sent again (decision 7). The only exceptions are the last sentence of decision 6. Every stop test has a time bound well under the wait it interrupts.
- **N4, never reacts to itself.** No marked event is recorded or counted as user input, and the program never presses a hotkey's key. The marker is a filter, not an identity check: another program could copy it.
- **N5, the keyboard is read only while needed.** Raw keyboard input is registered only while a recording runs, and while a hotkey's modifiers are still down after its activation (decision 9), when only the modifiers' state is read. Nothing outside a recording is logged or kept.

## X03: the spike, in the VM

X03 runs in the WN01 VM and ends with decision record 0006. It measures:

- SendInput clicks and keys reaching a test window and Minecraft Java (if it runs on the VM's OpenGL), counted as C02 counted them, at the 10 ms floor;
- interval accuracy at 10, 12 and 15 ms under tokio, with and without decision 5's settings, with all of the daemon's windows hidden (in a VM, so marked as an upper bound);
- what the marker and the raw-input device handle show for injected events, and whether a program can tell its own events from another injector's;
- whether raw input with `RIDEV_INPUTSINK` sees keys, buttons and relative motion while another window, including one reading raw input itself, has focus;
- whether `RegisterHotKey` fires while a game window has focus, whether the game also gets the keys, and whether a combination sent with SendInput fires it; how soon after an activation raw input reports the modifiers coming up;
- whether a release is delivered while an elevated window has focus and after a lock, and when it can be sent again;
- that a key pressed with SendInput stays down after the process is killed, and that a watchdog's release clears it, including after "End process tree" on the daemon;
- a topmost, click-through, non-activating window over a borderless window, taking no focus and no input.

## Slices

About 1,500 added lines each.

| Code | Slice | Blocked by |
| --- | --- | --- |
| WN01 | Windows 11 test VM on the dev box: scripts to build it from the evaluation image, autologon, OpenSSH, the test runner task, rebuild every 90 days | — |
| X03 | The spike above, decision record 0006 | WN01 |
| WN02 | Core on Windows: its file code ported (decision 12), core building and tested on Windows in CI | — |
| WN03 | The workspace on Windows: target-specific dependencies, the full CI job and cargo-deny target, Windows paths, language, log file, the daemon refusing to run elevated | WN02 |
| WN04 | Output backend: SendInput, scan codes, absolute moves, the marker, timing during runs, the per-system floor if X03 needs one | WN03, X03 |
| WN05 | The daemon on Windows: message thread, single instance, session lock and sleep, shutdown | WN03, X03 |
| WN06 | The watchdog, and releases sent again when Windows blocked them | WN04, WN05 |
| WN07 | The tray on Windows | WN05 |
| WN08 | Hotkeys on Windows, and never pressing a hotkey's key | WN06, WN07 |
| WN09 | Recording, screens and positions on Windows, recordings naming their source | WN06 |
| WN10 | The settings window, hotkey capture and rebinding, and the fixed-point picker on Windows | WN08, WN09 |
| WN11 | The on-screen display on Windows | WN09 |
| WN12 | Install, upgrade and uninstall script, guide and README for Windows | WN10, WN11 |

The clicker is usable at the cursor after WN08, with the watchdog in place; at a fixed point after WN10.

## Open questions, owned by slices

- WN02, settled 2026-11-24 (each with its test, in `crates/core/src/fs/`):
  - The checked directory on Windows: resolved once, refused if a reparse point, held open without delete sharing (decision 12). Tests: `windows/tests.rs` `a_held_folder_cannot_be_moved_deleted_or_replaced_by_a_junction`, `a_junction_on_the_way_swapped_after_the_check_never_receives_the_file`, `a_junction_in_place_of_the_folder_is_refused_and_nothing_is_written`.
  - The crate: none; std alone, and `nix` and `libc` are Unix-only dependencies of core.
  - Temporary files keep their name, `.<target>.<pid>.<16 hex>.tmp`, which no listing shows. New on both systems: the controller removes those an hour old or more at start, beside the recordings, the config and the current-recording file (an hour, since the single-instance name holds per login session), and every delete removes those in the recordings folder, so no hidden copy of a deleted recording stays. Tests: `temps.rs`, `controller/tests/recordings.rs` `a_start_removes_what_saves_that_crashed_an_hour_ago_left`, `manage/tests.rs` `a_delete_also_removes_what_crashed_saves_left`.
  - The retry after a sharing violation: inside the file layer, one budget of 300 ms for a whole save, listing or deletion (pauses of 10, 20, 40, 80 ms and the rest), then `ResourceBusy` saying another program holds the file; "access denied" on a change is retried too and stays "access denied" at the deadline. File work runs only while the daemon is idle. Tests: `patience.rs`, and on Windows `a_file_another_program_holds_is_busy_not_missing`, `a_file_let_go_within_the_wait_is_read`.
  - No-follow on the last component only, as on Linux; junctions further up are resolved once, as above.
  - Owner-only access: Linux keeps modes 0600 and 0700; on Windows the files take the permissions of the user's profile folder, and setting ACLs (Win32 calls) is left out. A user who loosens those permissions by hand opens the recordings too.
- WN04: absolute moves over a virtual desktop whose origin may be negative; the scan-code table's source and its test; undoing the timer settings on every way a run ends.
- WN05: how the message thread and the runtime wake each other without polling; removing the tray icon before the join deadline; the single-instance name in the session's namespace.
- WN06: what the daemon does when the watchdog dies or fails to start; ordering a previous watchdog's releases before a new daemon's first press; the table's size; passing the daemon's process handle by inheritance, never a PID; which handles the children inherit.
- WN08: the modifier wait's registration and its end on every way out; how the "never press a hotkey's key" rule, today tied to the portal route, applies to a rebindable route.
- WN09: format version or `positions_from` value for Windows positions; how mouse moves and positions are paired.
- WN10: renderer; how the overlay covers each monitor at its own DPI; keeping anything a dependency prints off the protocol pipe.
- WN11: whether the display stays click-through over exclusive fullscreen; what shows when it cannot.
- WN12: quoting the start-at-login path; an upgrade that asks the daemon to quit, waits with a deadline, stages the new files and leaves the old install untouched on failure.
