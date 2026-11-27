# 0007: What holds 10 ms clicks up on real Windows hardware

- Status: accepted, 2026-11-27 (WN06c, #135). It corrects what the [M5 brief](../planning/features/m5-windows.md)'s WN06 entry concluded about decision 7's check, and hands the open question to WN14 (#138).
- Measured on: the owner's test PC, reached with `scripts/windows-pc/pc` ([windows-pc.md](../agents/windows-pc.md)).
  - **Windows:** Windows 11 Pro, build 10.0.26300.9974, in Swedish, Balanced power plan.
  - **Machine:** an Intel Core i9-10980HK laptop (8 cores, 16 threads), on AC. Intel UHD Graphics, with a 1920×1080 panel at 300 Hz and 125 %. After the second reboot Windows also listed a Microsoft Basic Display Adapter.
  - **Code:** main at 70753a7. The tests and the receiver were built with the `test` profile, as the timing test runs; Rust stable (MSVC), and the versions `Cargo.lock` pins.
  - **Comparison:** the WN01 test VM, the same build.
- Tools: a lock-free trace, for measuring only and not merged. It records each click's run-timer wake, the task's poll, each `SendInput` with the thread's cycles and CPU, decision 7's check, and the press's arrival in the receiver window, on one performance-counter clock. Around it run interleaved 10 ms runs of output variants, each on a fresh thread with its own runtime. The patch, the traces and the analysis scripts are in the main checkout's `.claude/local/wn06c/wn06c-session/`.

## Decisions

1. **Decision 7's check stays as it is, around each release.** It is not what makes 10 ms clicks miss C02's bound on real hardware. The check costs 0.12 ms before the send and 0.27 to 0.32 ms after it at the median. It runs about 5 ms before the next press, and no press waits on it. In every batch where runs missed, the plain output missed as often as the check, or more.
2. **What makes periods late is a mouse button's own `SendInput`.** On this PC it takes about 0.5 ms (p50 495 to 508 µs per press in each arm of the traced runs b1 to b5), most of it waiting rather than working. In some spells it takes 1 to 16 ms. Most periods off by 1 ms or more are presses whose `SendInput` took 1 ms or more to return. The event reaches the window about 0.13 ms after the call returns at the median, and within 0.6 ms at every run's p99.
3. **The output path does not change.** Nothing found is in the program's reach there. Finding what a slow `SendInput` waits on, and whether the program can avoid it, is WN14 (#138).
4. **#135's first criterion is not met by a quiet spell.** In quiet spells the unchanged timing test passes with the check. In bad spells it fails with or without the check. So 30 passing runs at each interval would prove no fix, and the criterion is left unticked.

## Results

### `SendInput` on its own (`p50` per event, 1000 events each, 2 ms apart, into the receiver)

| Event | PC | VM |
| --- | --- | --- |
| Relative move by 0 | 164 µs | 37 µs |
| Left button, down or up | 561 µs | 113 µs |
| Key (F13), down or up | 121 µs | 27 µs |

### Interleaved traced runs at 10 ms (1000 clicks each)

| Batch | Plain output: runs missed | With the check: runs missed |
| --- | --- | --- |
| b1, just after a reboot (01:24) | 4 of 20 (138 periods off) | 2 of 20 (90 periods off) |
| b2, at BelowNormal and at Normal priority | 0 of 20 | 0 of 20 |
| b3 | 0 of 20 | 0 of 20 |
| b4, with eight busy loops at normal priority | 0 of 20 | 0 of 20 |
| b5, the display turned off or on before each run | 0 of 20 | 0 of 20 |
| b6, just after a second reboot (08:37) | 20 of 20 | 20 of 20 |

- **In b1,** 104 of the plain output's 138 periods off by 1 ms or more had a press `SendInput` of 1 ms or more, against 53 of the check's 90. Of the rest, 18 and 11 were presses that started late, 16 and 24 had neither a long send nor a late start, and 2 of the check's reached the window late after a short send.
- **In b6,** the PC was nearly idle: the test was the only process with noticeable CPU over the 8 minutes, and Explorer and Settings used under 2 s each. About a quarter of button `SendInput` calls took about 16 ms, one 60 Hz frame, and the p99 error was 13 to 22 ms in every run.
- **In b1 to b5,** the timer woke 0.17 to 0.22 ms after the deadline at the median and 0.65 to 0.68 ms at the median run's p99, and the task was polled 0.07 ms after that, in every arm. In b6 it woke 0.29 to 0.31 ms late at the median and 0.73 ms at p99, still far from the 16 ms sends.
- **Ruled out:**
  - CPU load (b4).
  - The priority class, in a quiet batch (b2). Tests started through `pc run` inherit the runner task's BelowNormal priority.
  - The display being off before a run (b5). A run turns the display on, and its first press then takes about 227 ms, a start the test leaves out.
  - The runtime thread becoming a GUI thread. Every arm converts it at its first `SendInput`.

### The unchanged timing test

| When | Runs missed |
| --- | --- |
| 26 Nov, main, 6 invocations (WN06's logs on the PC) | 0 of 54 (p99 at 10 ms 0.71 to 0.97 ms) |
| 26 Nov, WN06, 6 invocations | 5 of 54: 3 at 10 ms (1.19 to 1.53 ms), 2 at 15 ms (1.02 and 1.09 ms) |
| 27 Nov, 01:00, before the first reboot, 2 invocations | 1 of 18, at 10 ms (1.12 ms) |
| 27 Nov, after b2 to b5 | 0 of 27 (worst p99 0.80 ms) |
| 27 Nov, after b6 | 9 of 9, at every interval (p99 11 to 17 ms) |

On 26 November main's own p99 sat close to the 1 ms bound at 10 ms, so a spell of slower sends tipped 5 of WN06's 54 runs over. That is what the WN06 entry read as the check's cost.
