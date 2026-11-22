# O01: Show the countdown and what is running on the screen

- Issue: [#97](https://github.com/tobylo/auto-clicker/issues/97), a Feature after the 1.0 milestones (M0 to M4). Slices: O01a (#98), O01b (#99), O01c (#100).
- Maturity: refined 2026-11-22, sliced; O01a measured it the same day ([decision record 0005](../../decisions/0005-on-screen-display.md)): a game keeps its input under the display on Plasma, niri and Hyprland, so O01b and O01c go ahead. Rests on map D1, D5, D7 and D11, and on the Wayland backend of [W01](w01-other-desktops.md).
- Diagram: [where the on-screen display sits, and a recording from the tray step by step](https://prlens.dev/c/FmYhh3EhHk18kbkoilARyA).

## Problem and outcome

A start from the tray counts down so the user can switch to the game, but the seconds show only on the tray icon, which a fullscreen game covers. And once something runs, nothing on the screen says so: a recording or a clicker left running is easy to miss. With O01 the countdown shows as a large number in the middle of the screen, above the game, and a small still badge in a corner shows while the program clicks, records or plays. Each part has a switch in the settings.

The name is the **on-screen display**. "Overlay" already means the full-screen surface that picks the fixed point (U03).

## Scope

In scope: the countdown in the middle of every screen during the three tray countdowns; a badge in a corner of every screen while clicking, recording and playing; a `[on_screen]` config section; a group in the settings window; the tray's and the window's wording where the display is unavailable; the guide.

Out of scope: GNOME, which has no layer-shell; a countdown for hotkey starts, which start at once; elapsed time, blinking or animation on the badge; choosing one screen; hiding the display from screenshots and streams (the guide names the surface's namespace, so a compositor rule can).

## Invariants

These hold everywhere below and are stated once.

- **O1, one more reader, no second writer.** What shows is a function of three things and nothing else: the view's activity, the switches, and the screens. The display sends the controller nothing about what to do. Whether the desktop can show it reaches the view through the platform, as recording's availability does.
- **O2, never in the way of a run.** No start, stop, recording, save or shutdown waits on the display. Every request it makes of the compositor has its own deadline, and at most one attempt per screen is in flight, bound to what it was made to show: a change of what should show ends it, and an answer that comes after shows nothing. Removing never waits either: if a removal cannot be sent within its deadline, the display closes its connection, and the compositor removes the surfaces with it.
- **O3, a missing display keeps its reason, and failures keep their meaning.** Whether layer-shell is offered is read from the list of globals the layout's connection receives, at start and each time it connects again (W01's W5). Reading that list makes no layer-shell request, so O4 holds and nothing is opened for it (O5). A display that answered without layer-shell cannot show for as long as that connection lasts. No Wayland display, or one that did not answer, is unavailable with that reason, as the layout is. A surface the compositor closed or an answer that came too late is transient for its screen: that screen shows nothing for now and the others keep theirs. The display's connection, which all its surfaces share, being lost is transient for every screen. Either is tried again at the next countdown, when a screen is added or removed, or, for one screen, when its own size changes; a tick of the seconds only redraws a screen that shows, and retries nothing. After a lost connection the display's own connection can no longer report the screens, so that change is the layout's signal after the loss, which on Plasma fires when KWin is back (W5); on niri and Hyprland a lost connection comes back at the next countdown. A failure never triggers its own retry, and nothing retries on a timer.
- **O4, a connection of its own.** The display's surfaces and requests go on one connection of the display's own, never on the screen layout's. A protocol error ends the connection it happens on, and an unknown layout stops a playback that uses positions and a click run at a picked point.
- **O5, idle costs nothing, and showing costs next to nothing.** While nothing shows, nothing of the display exists: no surface, no buffer and no connection of its own. While it shows, the daemon wakes for it only on a change of the view, on its compositor's events (an answer to a request, a closed surface, a buffer released, a screen added, removed or resized), on the layout's signal after a lost connection (O3) and on the deadline of a request still unanswered. No frame callbacks, no animation and no periodic redraw.
- **O6, no input and no focus.** Each surface asks for no keyboard and has an empty input region, so the game keeps its keys, its clicks and its locked pointer. The display is not an input device, so nothing it does is recorded or read as a hotkey.
- **O7, every exit removes it.** Because of O1, everything that ends a run or a countdown (a stop, a run that finishes, an error, a locked session, sleep, a lost backend) removes the display by changing the view, and O2 says a removal cannot be held up. When the daemon exits or dies, the compositor removes the surfaces with the connection.
- **O8, what the compositor says is bounded.** The display shows on at most as many screens as a layout may hold (`MAX_OUTPUTS`). A surface's size has a fixed upper limit, and a buffer's size is computed with checked arithmetic. Past a bound, that screen is missing with the reason (O3).

## Behaviour

**The countdown (O01b).** While the view counts down to clicking, a recording or a playback, the whole seconds left show in the middle of every screen as digits alone: no tile and no backdrop behind them, and nothing dimmed. The digits take the job's colour (indigo; red for a recording) with a thin contrasting outline, so they read on light and dark scenes, and every other pixel of the surface is fully transparent. Minutes past 99 seconds show as on the tray icon. The number changes when the view does and goes when the run begins or the countdown is cancelled. The tray icon keeps showing it too.

**The badge (O01c).** While the view says clicking, recording or playing, a small badge sits in the chosen corner of every screen: a pointer mark, a red dot or a triangle. It shows however the run was started, the hotkeys included, and does not blink or count.

**Everything else shows nothing:** idle, a start waiting for the hotkey's modifiers to come up, and a start waiting for the devices to open, when the desktop's permission dialog may be on the screen.

**Screens.** One surface per screen, sized from that screen. Every screen, because the program cannot learn which screen the game is on, and a start from the tray leaves the desktop's attention on the tray's screen. While something shows, a screen that is added gets it and one that is removed loses it.

**Settings.** `[on_screen]` holds `countdown`, `clicking`, `recording` and `playback` (each on or off, on by default) and `corner` (`top_left`, `top_right`, `bottom_left` or `bottom_right`; `top_left` by default), parsed once with the rest of the file. The settings window shows them as a group. A save happens only while nothing runs, and the view carries the settings, so a change applies from the next show.

**Where it cannot work.** While the display is unavailable (O3), the tooltip lists it with the reason, as with recording (O01b), and the settings window's group is greyed out and says why (O01c).

**Screenshots and streams** of the game include the display. The surfaces carry one stable namespace, named in the guide.

## Decisions

Decided by the owner in refinement (2026-11-22):
1. **A badge while clicking too**, not only while recording and playing: a clicker left running is as easy to miss.
2. **Everything is on by default.** If O01a finds that a badge costs the game frames, the badges' default comes back to the owner before O01c.
3. **The badge's corner is a setting**, top left by default.
4. **Every screen**, not one the desktop picks or one chosen in the settings.

Defaults (reviewed; go ahead without asking):
5. Invariants O1 to O8.
6. **Mechanism:** layer-shell (`zwlr_layer_shell_v1`, the overlay layer), drawn by the daemon through the Wayland backend into shared memory, with the digits and marks drawn in code. The countdown is drawn by its rectangles, one sample per pixel or filled rows: the tray icon's 16 samples per pixel take 41 to 257 ms per screen at these sizes (0005). No Slint in the daemon, no helper process and no font. A surface the compositor owns goes when the daemon dies, which a helper process would not guarantee.
7. **An investigation first** (O01a, decision record 0005), since nothing else can say whether a game keeps its locked pointer when a surface appears and goes above it.
8. **Available by what the compositor offers**, never by a desktop's name (W01's W1). A desktop is refused by name only if O01a measures a failure there, and the owner decides that.
9. **Dependency:** `wayland-protocols-wlr` becomes a direct dependency of the Wayland backend. It is in `Cargo.lock` already through the settings window's winit, but the daemon is built on its own, so O01a measures what it adds to the binary, and says whether the shared-memory buffers need anything more. Measured (0005): the daemon grows by 148,912 bytes, and the buffers need nothing more: an anonymous file from rustix's `memfd_create`, already in the daemon, written without `mmap` or `unsafe`.
10. **Hotkey starts** get no countdown and do get the badge.
11. **Three slices:** the measurement; the display and the countdown behind its config key; the badges with the settings window's group. Each slice brings its own texts in English and Swedish and its part of the guide. The window's group waits for O01c so that O01b, which carries the fake compositor and its tests, stays near 1,500 lines; until then the window carries `[on_screen]` through a save unchanged, so a save there never resets a key set in the file.

Decided by the owner after the slices were opened (2026-11-22):
12. **The countdown is the digits alone**: no tile, no backdrop and nothing dimmed behind them. They take the job's colour with a thin contrasting outline (the default the owner was told about), and every other pixel is fully transparent (2026-11-22).

Decided by the owner after O01a's measurements (2026-11-22, [decision record 0005](../../decisions/0005-on-screen-display.md)):
13. **The badges stay on by default.** They cost Minecraft no frames that could be measured, at vsync on and a 120 fps cap. While a badge shows, the game on that screen is no longer scanned out directly on any of the three desktops (except perhaps on Intel, where KWin may keep the game on a plane of its own), and Hyprland stops tearing there; the guide says so and how to switch the badges off.
14. **Hyprland stays available.** On Hyprland an X11 test window that holds the pointer by warping it counted a nudge of a few pixels when the display appeared, changed or went; native games and Minecraft showed none. The window's counting could produce the same numbers, so the nudge is to be measured again on Hyprland before the guide names it or anything is reported upstream.

## Acceptance criteria

1. A start from the tray counts down in the middle of every screen, above a fullscreen game, and the number goes when the run begins or the countdown is cancelled (O01b).
2. A badge shows in the chosen corner of every screen while clicking, recording and playing, and goes with every way a run ends (O01c).
3. With the display showing, the game keeps the keyboard, the clicks at the display's place and its locked pointer, measured on Plasma, niri and Hyprland (O01a).
4. Each switch turns its part off and on, and the corner moves the badge, from the next show after a save (O01b, O01c).
5. A display that is unavailable says why in the tooltip, and one that is lost, refused or late never delays, stops or fails a start, a stop, a recording, a save or shutdown (O01b). The settings window's switches are greyed out with the reason then (O01c).
6. Idle cost unchanged, and while the display shows the daemon wakes for it only as O5 allows (measured in O01b and O01c).

## Validation

Unit tests against a fake compositor and the fake backend, on the paused clock, with an upper time bound on every "never delays" test. Hand checks on Plasma 6.8 (two screens), niri and Hyprland, with the versions in each PR. O01a's measurements need the machines: Minecraft holding a pointer lock on each desktop needs the owner there, and Hyprland runs as the only session.

Measured by O01a on 2026-11-22 ([decision record 0005](../../decisions/0005-on-screen-display.md)): on Plasma 6.8.3 (two screens), niri 26.04-291 and Hyprland 0.57.2, a fullscreen game kept its keyboard focus, keys, clicks at the display's place and locked pointer while the countdown and the badge appeared, changed and went, natively and under XWayland, for test windows and for Minecraft played by hand, with no frame-rate change. Read from source only: the lock screen on niri and Hyprland, a screen unplugged live on Plasma, and VRR and tearing switched on. Not measured: sleep (the laptop crashed entering it) and Hyprland's sharpness at 1.6 (no capture tool).

## Open questions

For O01a: settled by decision record 0005.

Settled by O01b (2026-11-22), each with its test (in `crates/backend-wayland/src/display/`, `client/surfaces.rs` and `crates/core/src/digits.rs` unless named):

- **Size and look.** Characters a quarter of the screen's logical height, smaller where the label and its padding would pass 90 % of the screen's width ("99m" on a portrait screen); a white outline of 2.5 % of the characters' height, at least a pixel; a quarter of the height as padding; no anchor, so the compositor centres the surface. `fit::tests`, `paint::tests::the_buffer_holds_the_digits_and_their_outline_and_nothing_else`.
- **The buffer at a fractional scale.** round(logical × scale / 120) per side from `wp_fractional_scale_v1`, with checked arithmetic, shown at the logical size through `wp_viewporter`; drawn at 120 until the scale arrives, then again; without both globals, at scale 1. `the_buffer_follows_the_screens_fractional_scale_at_its_logical_size`.
- **Screens added, removed and resized** are heard on the display's own connection, each screen's size read once its own update ended, from the connection's setup on, so one odd screen leaves the others showing; a screen that showed nothing is tried again when a screen is added or removed or its own size changes; after a lost connection, only the layout's signal after the loss brings it back. `a_screen_added_while_it_shows_gets_it_and_one_removed_loses_it`, `one_screen_that_never_ends_its_update_leaves_the_others_showing`, `a_screen_that_changes_size_is_tried_again`, `a_lost_connection_comes_back_with_the_layouts_signal_and_not_before`, `a_screen_change_heard_while_it_showed_does_not_bring_a_lost_connection_back`.
- **Each connection reaches the daemon's own display**: it is checked against the listener found at start. `a_display_another_process_listens_on_is_refused_at_once_and_not_retried`.
- **Shared memory** from `memfd_create`, sized, written with `pwrite` and sealed, closed once the pool is made (both Wayland backends duplicate a sent descriptor); no `mmap` and no `unsafe`. The hand checks on Plasma, niri and Hyprland draw through it.
- **Where a failure's reason is kept.** Why the desktop cannot show at all (no display, one that did not answer, no xdg-output, no layer-shell) is read from the layout's connection at start and each time it connects again, and reaches the view and the tooltip while the countdown is switched on. A closed surface, a late answer, a lost connection and a screen past a bound go to the log, and a screen's reason stays in the display's own state for that countdown. `why_the_display_cannot_show_is_what_the_layouts_connection_found`, `the_layouts_connection_made_again_says_anew_whether_the_display_can_show`, `controller::tests::on_screen`, `screens_past_the_bounds_show_nothing_and_keep_the_reason`.
- **What bounds the daemon's one thread.** The connection's setup has 1 s and each surface's answer its own 1 s, and an answer read after its deadline is late; a new size asked while a surface's answer is awaited takes a new surface, so the old answer reaches nothing; the digits are filled row by row; a surface's and a buffer's side is at most 4096 px (64 MiB); a screen holds at most two buffers the compositor has not released, and draws again only after a release; one screen is drawn per poll, the view read again between screens. With that, what the display sends between two reads is a few requests a screen, far below the socket's buffer, so a removal is never held back by a compositor that stopped reading, and the connection is dropped when the countdown ends. `a_display_that_never_answers_its_setup_is_left_at_once_when_the_countdown_ends`, `a_closed_surface_or_a_late_answer_on_one_screen_leaves_the_other_showing`, `an_answer_read_after_its_deadline_shows_nothing`, `a_new_size_asked_before_the_first_is_answered_takes_a_new_surface`, `a_screen_holds_two_buffers_until_the_compositor_releases_one`.
- **The digits** move to core (`auto_clicker_core::digits`), shared by the tray icon, which keeps its tile and 16 samples, and the display.
- **The retry rule** (O3 above): a tick only redraws; a failed screen or connection is tried again at the next countdown or a screen change, never on a tick or a timer.
- **A connection per countdown.** The display connects when a countdown starts and drops the connection when it ends, which removes every surface with it: no removal is ever waited for, and shutdown removes it the same way. `a_cancelled_countdown_removes_the_number_at_once`, `an_answer_after_the_countdown_ended_shows_nothing`, `shutdown_closes_the_displays_connection`.

The display's tests run against a fake compositor on real time with generous bounds, as the cursor sessions' do: a paused clock would let their socket waits time out at once.

A daemon that hangs while a badge shows leaves the badge up until it is killed. Accepted: the same hang leaves the tray icon wrong, and killing the daemon removes both. A compositor that hangs is not the display's to solve: O2 closes the connection, and the surfaces go when the compositor reads again.
