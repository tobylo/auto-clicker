# 0006: Windows input, timing, hotkeys, recording and release on death

- Status: accepted, 2026-11-25 (X03, #109). It settles what the [M5 brief](../planning/features/m5-windows.md) assumed about Windows, and the open questions of the slices it affects (WN04 to WN06, WN08, WN09, WN11 and WN12).
- Measured on: the WN01 test VM on the dev box (`scripts/windows-vm/vm`).
  - **Windows:** Windows 11 Enterprise Evaluation 26H2, build 10.0.26300.9974. The registry's `CurrentBuild`/`UBR` give 26300.9974. The desktop watermark reads "26100.ge_release" and the registry's `ProductName` still says "Windows 10", both left over from the base image. Balanced power plan, en-US input. UAC is on (`ConsentPromptBehaviorAdmin` 5, `PromptOnSecureDesktop` 1), and "Enhance pointer precision" is on.
  - **Machine:** QEMU 11.1.3 (KVM, q35, 8 vCPUs, Hyper-V enlightenments including `hv_stimer`), on Linux 7.3.5-1-cachyos. Plain VGA at 1920x1080 and 100 %. Input comes from a PS/2 keyboard, a USB tablet and a PS/2 mouse. The dev box's load average was 3.5 to 5.9 on 32 threads during the runs.
  - **Toolchain:** Rust 1.100.0 (MSVC), windows-sys 0.61.2 and tokio 1.54.1, the versions `Cargo.lock` pins.
- **VM-only:** timing, a real game, and fullscreen behaviour on a real graphics card can differ on real hardware. Every figure here is from the VM and is marked as an upper bound where hardware could do better.
- Tools: a throwaway probe (`x03.exe`, one actor per mode) and one PowerShell scenario per measurement, kept under the main checkout's `.claude/local/x03/` with a README that says how to run each. Each scenario's logs are in `.claude/local/x03/results/<scenario>-<time>/`. Every probe process timestamps on QueryPerformanceCounter, one clock for all of them.

## Decisions

1. **Injecting by scan code holds (brief decision 2).** For every key tested, SendInput by scan code, with the extended flag on E0 keys, gave the window and raw input the same scan code, extended bit and virtual key as QEMU's real keyboard. NumLock is the exception that must be sent without the extended flag. Pause cannot be sent by scan code (it is an E1 sequence) and goes by virtual key. Point 1.
2. **The 10 ms floor holds on Windows, with the run's deadlines timed by a high-resolution waitable timer, not by tokio's sleep (brief decision 5, changed: see "Decisions changed").**
   - **tokio's sleep:** with 1 ms timer resolution and the power-throttling opt-out, tokio's sleep wakes 0 to 2 ms late on every deadline. About 22 % of 10 ms periods were off by 10 % or more, and 5 % at 12 ms. C02's criterion, which this record applies to Windows too, is that a period is off by 10 % or more only at a run's start or in a rare scheduling stall, and that the 99th-percentile error stays under 1 ms. That comes from how tokio's sleep waits on Windows (whole milliseconds and the system tick), not from noise.
   - **The waitable timer:** a high-resolution waitable timer on a thread of its own, waking the runtime through a channel, missed 0.2 to 0.4 % of periods, all at start-up or in one hypervisor stall. Its 99th-percentile error was 0.41 ms, and it needs no raised timer resolution at all. Point 2.
3. **Our own events are told apart by the marker alone, and the marker fits in 32 bits (brief decision 8).** Raw input gives every injected event, ours or another program's, device handle 0, and gives real devices their own handle. It carries only the low 32 bits of the extra information, while window messages carry all 64. Point 3.
4. **Raw input with `RIDEV_INPUTSINK` sees what recording needs, as far as the VM can show (brief decision 10 stands, no hooks).** It saw keys and buttons from real devices, and keys, buttons, relative and absolute moves from another program. That held behind every kind of window in front tried: a plain window, a game-like window reading raw mouse, one reading raw keyboard too, one registered with `RIDEV_NOLEGACY`, one with `RIDEV_NOHOTKEYS`, and, for real keys, an elevated window. Relative motion from a real mouse could not be produced in the VM and waits for real hardware. Point 4.
5. **`RegisterHotKey` with `MOD_NOREPEAT` works with a game in front (brief decision 9 stands), with one limit.** It fired once per press, from the real keyboard and from SendInput, so the rule never to press a hotkey's key is needed. The game gets Ctrl, Shift and F6's release, but not F6's press. The modifier wait registered within 0.24 ms and never missed a release, even with all three keys up within 0.5 ms.
   - **The limit:** a window in front that registers its keyboard with `RIDEV_NOHOTKEYS` turns every registered hotkey off (0 of 5), and the game gets F6 instead. So during a run the stop hotkey is also read in raw input (see "Decisions changed"). Point 5.
6. **A blocked release is sent again, and the signals for "again" exist (brief decision 7 stands).**
   - **Under an elevated window,** SendInput reports success but the release does not arrive. Every unelevated program then reads all keys as up. The foreground event, with the foreground process's integrity level, which an unelevated program can read, says when an unelevated window has the foreground again.
   - **On the lock screen and the UAC prompt,** SendInput fails openly (error 5). The system's key state reads up from the switch on, yet the window in front gets no key-up. The desktop-switch event, and for the lock the session notice, mark the return.
   - **In every case,** sending the release again afterwards delivered it. Point 6.
7. **The watchdog releases what a killed daemon held, but must also send again (brief decision 6, changed: see "Decisions changed").** A key and a button pressed by a killed process stayed down for 30 s, until something released them. A watchdog parent survived `taskkill /T` and a kill by name and released within 0.5 ms. Task Manager runs elevated for an administrator, so a kill from it leaves Task Manager in front. There the watchdog's release reported success, was dropped, and the key stayed down until a second release. Point 7.
8. **The on-screen display's window type takes nothing from the game (brief decision 15's display, WN11).** A topmost, layered, click-through, non-activating window took no focus, no click (even on its opaque edge), no key and no motion from a plain or game-like window under it. A topmost game window activated after it covers it, and re-asserting `HWND_TOPMOST` without activating brings it back. Point 8.

## Decisions changed

Two results changed the brief's decisions. The owner settled both on 2026-11-25, and the brief says them as decisions 5 and 6.

- **Decision 5, timing.** On Windows, a run's deadlines are waited for by a high-resolution waitable timer on a thread that exists only while a run is active. It wakes the runtime, and a stop or shutdown ends its wait at once. Nothing raises the system's timer resolution. The execution-speed power-throttling opt-out stays, during runs only. The 10 ms floor stays on both systems. The alternative was to keep tokio's sleep at 1 ms resolution and make the floor 20 ms on Windows.
- **Decision 6, the watchdog sends again.** The daemon clears a table entry only once its release counts as delivered under decision 7, and the watchdog applies the same rule to its own releases. A release counts as delivered only when the user's desktop had input and the window in front was not of higher integrity, both before and after the send. Any other release stays in the table and is sent again on the next foreground or desktop change. The watchdog exits only when every entry is delivered. That wait has no deadline: like the daemon's idle wait, it sleeps on events and ends at delivery or when the session ends. AGENTS.md's rule that every wait is bounded names it as its second exception.
- **Decision 9 and invariant N5, stopping a run.** While a run is active, the backend also reads raw keyboard input and matches only the combinations of the hotkeys that stop the running job, the same set as on Linux. So a run stops by its hotkey even when a game in front has registered `RIDEV_NOHOTKEYS`. The alternative was to name such games in the guide and stop runs from the tray.

## How it was measured

Each measurement is one `vm run` of a scenario script, which starts every process it needs in tester's desktop. Real input, which Windows treats as hardware, comes from QEMU:
- **Keyboard:** the PS/2 keyboard, through the monitor's `sendkey`. It was exact for every key tried.
- **Buttons:** the USB tablet's buttons. A tablet button press reports absolute (0, 0) in raw input, but Windows does not move the pointer, so the click lands where the probe last put the pointer with `SetCursorPos`.
- **Not available:** the monitor's `mouse_move` cannot place the tablet; it always goes to the PS/2 mouse. Every PS/2 move, of any size, arrived in raw input as (508, 508), so no real relative motion could be produced. Relative motion from a real mouse is therefore unmeasured. Relative motion from another program's SendInput, which brief decision 8 counts as the user's, stands in for it.

`vm key` waits on the lock that the running `vm run` holds. So a dev-box script (`drive.py`) reads which request the scenario has reached from a 16x16 marker the probe draws in the screen's corner, read through `screendump`, and sends the input straight to the monitor socket. A scenario checks that each piece of real input arrived (in its own logs) before it reads a result. See "Tooling found broken" for why this script reads the screen rather than a file over SSH.

## Commands

From the main checkout's `.claude/local/x03/`, after `./push.sh` (copies `probe/` and `scen/` into the VM and builds there with `cargo build --release --locked`). `run.sh SCENARIO [--elevated] TIMEOUT [ARGS]` starts `drive.py` and runs `scripts/windows-vm/vm run [--elevated] --timeout TIMEOUT -- powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File C:\work\x03\scen\SCENARIO.ps1 [ARGS]`, then fetches the logs. `analyze.py clicks|timing DIR` gives point 1's and 2's figures.

| Point | Command | Scenario |
| --- | --- | --- |
| 1, clicks | `./run.sh m1 300` | `scen/m1.ps1` |
| 1, keys | `./run.sh m1keys 200` | `scen/m1keys.ps1` |
| 2 | `./run.sh timing 1800`; the control, with no driver running: `../../../scripts/windows-vm/vm run --timeout 600 -- powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File 'C:\work\x03\scen\timing.ps1' -Intervals 10 -Runs 3 -Settings 'throttle,waitable'`; row (f), likewise: `../../../scripts/windows-vm/vm run --timeout 900 -- powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File 'C:\work\x03\scen\timing.ps1' -Intervals '10,12,15' -Runs 3 -Settings bridge` | `scen/timing.ps1` |
| 3 | `./run.sh m3 200` | `scen/m3.ps1` |
| 4 | `./run.sh m4 300` | `scen/m4.ps1` |
| 5 | `./run.sh m5 400` | `scen/m5.ps1` |
| 6 | `./run.sh uipi --elevated 200`; `./run.sh lock 300`; `./run.sh uac 300` | `scen/uipi.ps1`, `lock.ps1`, `uac.ps1` |
| 7 | `./run.sh watchdog 300` | `scen/watchdog.ps1` |
| 8 | `./run.sh overlay 300` | `scen/overlay.ps1` |

The VM was reset (`vm stop && vm reset`) before the timing runs.

## Results

### 1: clicks and keys reach a window (measured)

- **Clicks** (`m1-173239`): 1000 SendInput clicks at 10 ms reached a game-like window (borderless, full screen, pointer clipped and hidden, mouse through raw input) 1000 of 1000 times, both in raw input and as `WM_LBUTTONDOWN`. They reached a plain window 1000 of 1000 times too. In M2, 45,000 more clicks all arrived.
- **Keys** (`m1keys-173515`), injected by scan code and compared with QEMU's real keyboard pressing the same keys:

| Key | Scan code sent | Raw input (injected = real) | Window message (`lParam`) |
| --- | --- | --- | --- |
| A, LCtrl, RShift, F6, keypad 1 | 1E, 1D, 36, 40, 4F | the same make code, no E0, VK 41, 11, 10, 75, 61 | the same |
| RCtrl, Left, Up, keypad Enter, Insert, Delete, Home, keypad / | the code with `KEYEVENTF_EXTENDEDKEY` | make code with E0 (flag 2), the right VK | extended bit set, the same scan code |
| RAlt | 38, extended | E0 38, VK 12, `WM_SYSKEYDOWN` | extended, `WM_SYSKEYDOWN` |
| NumLock | 45, **not** extended | make 45, no E0, VK 90, as real | real sets the extended bit and injected does not; sent as 45 with the extended flag, it came through as VK FF (unknown) |
| Pause | not expressible by scan code | real: E1 1D (VK 13), then 45 (VK FF); by VK 13: make code 0, VK 13 | VK 13 both ways; scan code 45 real, 0 injected |
| PrintScreen | 37, extended | either way, Windows opened the Snipping Tool ("Select area to snip") and took the foreground | |
| Left Windows | 5B, extended | either way, Windows opened Start and took the foreground | |
| Applications | 5D, extended (injected) | E0 5D, VK 5D | QEMU's `menu` key sent nothing to compare with |

### 2: interval accuracy (measured, VM-only)

`timing-174027`, `timing-control-175224` and `timing-bridge-175424`, from a freshly reset VM. Each row is three runs of 1000 clicks. The clicking process had no visible window (a hidden one, as the daemon's are). A plain window in front counted every click and stamped its arrival. "Off" counts periods 10 % or more from the interval, C02's criterion (2,997 periods in all). "Late p99" is the 99th percentile of each press against the absolute schedule. `Sleep(1)` is the mean of 20 calls during the run.

| How the deadlines are waited for | 10 ms: off | 12 ms: off | 15 ms: off | p99 error per run (10 ms) | late p99 | `Sleep(1)` |
| --- | --- | --- | --- | --- | --- | --- |
| (a) tokio's sleep, nothing set | 2,997 (100 %) | 2,995 | 174 | 9.8 ms | 15.3 ms | 15.8 ms |
| (b) as (a), with `timeBeginPeriod(1)` | 630 (21 %) | 144 | 102 | 1.5 to 2.0 ms | 2.0 ms | 1.8 to 2.0 ms |
| (c) as (b), with the power-throttling opt-out (execution speed and timer resolution) | 672 (22 %); control 662 | 144 | 74 | 1.5 ms | 2.0 ms | 1.9 ms |
| (d) as (b), with a visible topmost window | 674 (22 %) | 148 | 81 | 1.5 ms | 2.0 ms | 1.9 ms |
| (e) high-resolution waitable timer (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`), plain thread, nothing set | 47; control 9 | 21 | 16 | 0.5 ms | 0.5 ms | 15.8 ms (unchanged) |
| (f) as (e), its thread waking tokio through a channel | 12 (0.4 %) | 7 | 8 | 0.41 ms | 0.52 ms | 15.8 ms (unchanged) |

- **All 45,000 clicks of the first run arrived,** and the mean period was exact in every row.
- **(a):** follows the system's 15.6 ms tick, so 10 and 12 ms cannot be kept.
- **(b) to (d):**
  - The wakeups are spread evenly between 0 and 2 ms late, so a fifth of 10 ms periods are off by 1 ms or more. That is a property of how tokio's sleep waits on Windows: whole milliseconds, on top of a 1 ms tick that wakes up to a tick late.
  - It is not noise. The control run, with the input driver stopped and the window logging only to memory, gave the same 22 %.
  - Hidden or visible made no difference, and neither did the opt-out. In this VM, Windows honoured `timeBeginPeriod` for a process with only a hidden window. The map's fact that Windows 11 ignores it there was not seen here (VM-only: it depends on power management, which a VM on a Balanced plan may not apply).
- **(e) and (f):** the misses are the first press of each run, which comes 12 to 18 ms late and is then caught up, plus one or two stalls of 7 to 60 ms in some runs. The stalls are irregular and appear in every setting, so they are the VM's scheduling (VM-only). They are not harness noise: the control showed none outside start-up. Leaving out each run's first three periods (the late first press and its catch-up), (f) at 10 ms missed 3 of 2,991, all in one stall.
- **The setting is undone:** in (b) to (d), `Sleep(1)` was back to 15.0 ms in the same process after `timeEndPeriod(1)`. A second process asking for nothing measured 15.8 ms throughout.

### 3: the marker and the device handle (measured)

`m3-175701`. A popup window in front read window messages and raw input; a hidden sink read raw input in the background.

| Source | Raw input `hDevice` | Raw input extra information | `GetMessageExtraInfo` |
| --- | --- | --- | --- |
| Injector A, marker `0x5A5A00004143FFFF` | 0 | `0x4143FFFF` (low 32 bits) | `0x5A5A00004143FFFF` |
| Injector B, no marker | 0 | 0 | 0 |
| Injector B, marker `0x1234` | 0 | `0x1234` | `0x1234` |
| QEMU keyboard | `0x1003F` | 0 | 0 |
| QEMU tablet buttons | `0x10048` | 0 | 0 |

The same held for keys, buttons, relative moves and absolute moves. An absolute move is reported in raw input as absolute, with its normalised coordinates. So a program tells its own events from another injector's only by the marker. A marker the recorder compares in raw input must fit in 32 bits.

### 4: raw input in the background (measured)

`m4-175729`. A hidden sink registered keyboard and mouse with `RIDEV_INPUTSINK`. In each case, injector B sent a key, a click, ten relative moves (+3 px) and three absolute moves, then QEMU's keyboard sent a key and the tablet a click.

| Window in front | Sink: keys, buttons, relative moves (sum), absolute, real key and click | Front window |
| --- | --- | --- |
| popup (window messages only) | 2/2, 2/2, 10 (30), 3/3, 2/2 and 2/2 | every key and click |
| game-like: raw mouse, pointer clipped | the same | every key and click, raw buttons 4 |
| game-like, raw keyboard too | the same | the same, raw keys 4 |
| game-like, `RIDEV_NOLEGACY` on both | the same | raw input only, all of it |
| game-like, `RIDEV_NOHOTKEYS` | the same | every key and click |

- **Relative motion:** raw input reports the raw counts (30). The pointer moved 22 px, through pointer acceleration.
- **An elevated window in front** (from point 6's run, `uipi-180053`): the unelevated sink still received the real keyboard's key events while the elevated window had the foreground.
- **Positions (for WN09):** at each absolute move, `GetCursorPos` read in the sink's `WM_INPUT` handler still gave the position before that move (3 of 3, 100 ms apart). The pointer is moved after raw input is delivered, so a position read at `WM_INPUT` belongs to the previous event.

### 5: `RegisterHotKey` with a game in front (measured)

`m5-175844`. A hidden window registered Ctrl+Shift+F6 with `MOD_NOREPEAT`.
- **Sources:**
  - injector B, releasing in reverse order 80 ms after F6;
  - injector B, all six events back to back;
  - injector B, F6 repeated three times while held;
  - QEMU's keyboard, holding F6 for 100 ms;
  - QEMU's keyboard, holding F6 for 10 ms, with the modifiers released 10 ms apart.
- **Results by window in front:**
  - **Game-like windows (raw mouse; raw keyboard; `RIDEV_NOLEGACY`; topmost):** `WM_HOTKEY` came once per press from every source, so `MOD_NOREPEAT` held. The game got Ctrl down, Shift down and F6 up, and then Shift and Ctrl up. It got no F6 down, neither as a window message nor in raw input.
  - **`RIDEV_NOHOTKEYS` on the game's keyboard:** no `WM_HOTKEY` from any source (0 of 5). The game got every key, F6 down included.
- **The modifier wait:** on `WM_HOTKEY`, raw keyboard input registered within 0.06 to 0.24 ms, then Ctrl and Shift were read once with `GetAsyncKeyState`. Each release came through when it happened: the last 30 ms after the hotkey for the real 10 ms press, and 0.2 to 0.6 ms for the instant injected one. Not one wait missed its release, including the instant case, where Shift was already up when the wait registered.

### 6: releases Windows blocks (measured)

Each case: an unelevated injector pressed F13 and the left button into an unelevated window, a change took the input away, the injector sent the releases, and then sent them again once the change was undone.

| Case | SendInput while blocked | Key state while blocked | The window got the release | The signal it is back | Sent again |
| --- | --- | --- | --- | --- | --- |
| Elevated window in front, by real Alt+Tab (`uipi-180053`) | 1 inserted, no error | the elevated window read F13 and the button down throughout; unelevated programs read all keys up | no | `EVENT_SYSTEM_FOREGROUND` to an unelevated window (an unelevated program reads the elevated window's integrity, "high") | delivered, all up |
| Lock, by `LockWorkStation` (`lock-181022`) | 0 inserted, error 5 | every program read up from the lock on | no, not even after the unlock | `WTS_SESSION_UNLOCK`, then `EVENT_SYSTEM_DESKTOPSWITCH` with `OpenInputDesktop` succeeding again, 20 ms apart | delivered: the window's `WM_KEYUP` and `WM_LBUTTONUP` came only now |
| UAC prompt (`uac-181121`) | 0 inserted, error 5 | read up | no | `EVENT_SYSTEM_DESKTOPSWITCH` with the input desktop back; no session notice | delivered |

- **After the lock and the prompt,** the system's key state read up although the window had never seen the key come up. A game keeps holding a key Windows reports as up, so the program cannot use key state to decide whether to send again.
- **Under an elevated window,** an unelevated program's `GetAsyncKeyState` reads every key as up. So that reading means nothing either.

### 7: a killed process and the watchdog (measured)

`watchdog-181251`. The holder pressed F13 and the left button, recording each in a table before it sent the press. The watchdog is a separate executable, the holder's parent, holding the holder's process handle and the table's mapping.

| Case | Result |
| --- | --- |
| Holder alone, `Stop-Process -Force` (TerminateProcess) | F13 and the button stayed down at 1, 5 and 30 s: the window got no release and the system's key state read down until the probe released them |
| Watchdog parent, `taskkill /F /T` on the holder (the brief's "End process tree" on the daemon) | the watchdog survived, read the table and released both 0.46 ms after the holder ended; the window got `WM_KEYUP` and `WM_LBUTTONUP` |
| Watchdog parent, `taskkill /F /IM x03-holder.exe` | the same, 0.39 ms |
| Task Manager in front (started unelevated, it ran elevated with no prompt), `taskkill /F /T` on the holder | the watchdog's releases returned 1 inserted, no error, but did not arrive. Once Task Manager was closed, the window still had F13 and the button down. A second release from an unelevated process delivered them |

- **Task Manager's own "End process tree":** not driven through the menu here. The `taskkill /T` case gives the same process outcome (read, not measured). Whatever kills the daemon from Task Manager, the user has Task Manager in front at that moment, which is the dropped-release case.

### 8: the display's window over a borderless window (measured; fullscreen VM-only)

`overlay-181409`. The overlay was a 200x200 window: `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, per-pixel alpha, an opaque 20 px ring around a transparent middle, shown with `SW_SHOWNOACTIVATE`. It appeared, changed colour three times and went within 3 s, while injector B streamed relative moves at 1 kHz (alternating ±1), a right Shift tap every 250 ms and a click every 500 ms, with the pointer on the opaque ring at (870, 450). Each case had a control without the overlay first.

| Window under it | With the overlay | Control |
| --- | --- | --- |
| full-screen popup (window messages) | 6 of 6 clicks at (870, 450), 12 of 12 keys, 3,000 moves summing to 0, no focus event | the same |
| game-like (raw mouse, clipped) | 6 of 6 clicks (raw and messages), 12 keys, 3,000 moves, no focus event | the same |
| game-like and topmost, activated after the overlay | the overlay went under it (screen capture). Re-asserting `HWND_TOPMOST` with `SWP_NOACTIVATE` brought the overlay back on top, and the game kept focus and every input | — |

The overlay was never the foreground window. Exclusive fullscreen could not be tried: the VM's VGA adapter has no flip-model fullscreen. That question stays open for real hardware (WN11).

## Not measured here

- **Minecraft and any real game:** the VM's network reaches only Rust's and crates.io's hosts, code reaches it only from this repository, and the game needs the owner's account. The game-like window above (borderless, pointer clipped and hidden, mouse through raw input, keys from the scan code, as SDL reads them) stands in, VM-only.
- **Exclusive fullscreen, and timing without a hypervisor:** real hardware.
- **Relative motion from a real mouse:** QEMU's PS/2 moves arrive as garbage (see "How it was measured").
- **Absolute moves across several monitors or a negative origin, and DPI other than 100 %:** the VM has one 1920x1080 screen.
- **DirectInput:** windows-sys has no COM interfaces. SDL and GLFW read the paths measured here.

## Tooling found broken (reported, not fixed here)

- **SSH:** a second, long-lived SSH connection to the VM (`Get-Content -Wait` on a request file) made new connections fail with "Connection timed out during banner exchange" for minutes, `vm run` among them. Short connections in parallel worked. The cause was not found. The probe's driver reads the screen instead.
- **Mouse input:** the QEMU monitor's `mouse_move` always moves the PS/2 mouse, even after `mouse_set` selects the tablet, so a session cannot place the pointer. Every PS/2 move arrives in Windows as (508, 508), whatever was sent.
- **`vm key`:** it takes the VM's lock, so a session cannot press a key while its own `vm run` is running.
