//! One bound session: what its signals become.

use super::*;

#[test]
fn a_bind_answer_gives_each_action_its_trigger_and_no_trigger_to_the_rest() {
    let triggers = bound_triggers(&listed(&[
        (RECORDING, "Meta+F7"),
        ("something_else", "Ctrl+F1"),
    ]));
    let mut expected = Triggers::unknown();
    expected.set(Action::ToggleClicking, Trigger::reported(""));
    expected.set(Action::ToggleRecording, Trigger::reported("Meta+F7"));
    expected.set(Action::TogglePlayback, Trigger::reported(""));
    assert_eq!(triggers, expected);
}

#[tokio::test(start_paused = true)]
async fn a_press_fires_at_once_and_the_release_comes_a_pause_after_letting_go() {
    local(async {
        let s = Served::start().await;
        let start = Instant::now();
        s.press(CLICKING).await;
        assert_eq!(s.take(), [fired(Action::ToggleClicking)]);
        assert_eq!(start.elapsed(), Duration::ZERO);
        // Held: no release, however long.
        advance(Duration::from_secs(20)).await;
        settle().await;
        assert_eq!(s.take(), []);
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE - Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(s.take(), [], "released before the pause was over");
        advance(Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(s.take(), [HotkeyEvent::ModifiersReleased]);
        // Once.
        advance(Duration::from_secs(60)).await;
        settle().await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn every_action_fires_by_its_id_and_unknown_ids_do_nothing() {
    local(async {
        let s = Served::start().await;
        for (id, action) in [
            (CLICKING, Action::ToggleClicking),
            (RECORDING, Action::ToggleRecording),
            ("toggle_playback", Action::TogglePlayback),
        ] {
            s.press(id).await;
            s.let_go(id).await;
            assert_eq!(s.take(), [fired(action)], "{id}");
        }
        s.press("clicking").await;
        s.press("").await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn every_activation_fires() {
    local(async {
        // The desktop sends one per press and releases a shortcut before
        // it activates another; nothing is held back on a guess.
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        s.press(RECORDING).await;
        s.let_go(RECORDING).await;
        s.press(CLICKING).await;
        assert_eq!(
            s.take(),
            [
                fired(Action::ToggleClicking),
                fired(Action::ToggleRecording),
                fired(Action::ToggleClicking)
            ]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stray_release_after_the_reported_one_reports_nothing_more() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(
            s.take(),
            [
                fired(Action::ToggleClicking),
                HotkeyEvent::ModifiersReleased
            ]
        );
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE * 2).await;
        settle().await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_press_that_arrives_with_a_due_release_comes_first() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        // The second press is queued when the pause runs out: it is handled
        // before the release, which then belongs to it and waits for its
        // own letting go.
        s.signals
            .send(Signal::Activated(CLICKING.to_owned()))
            .unwrap();
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(
            s.take(),
            [fired(Action::ToggleClicking), fired(Action::ToggleClicking)]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_second_press_during_the_pause_shares_one_later_release() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE / 2).await;
        // Pressed again before the release was reported: a cancel.
        s.press(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(
            s.take(),
            [fired(Action::ToggleClicking), fired(Action::ToggleClicking)],
            "the first press's release was reported while the second was held"
        );
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(s.take(), [HotkeyEvent::ModifiersReleased]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_release_without_a_press_reports_nothing() {
    local(async {
        let s = Served::start().await;
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE * 3).await;
        settle().await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn changed_triggers_are_reported_whole_with_the_change_applied() {
    local(async {
        let s = Served::start().await;
        s.hear(Signal::Changed(listed(&[(RECORDING, "Meta+F11")])))
            .await;
        let mut expected = bound_triggers(&defaults());
        expected.set(Action::ToggleRecording, Trigger::reported("Meta+F11"));
        assert_eq!(s.take(), [HotkeyEvent::Triggers(expected.clone())]);
        // A trigger removed in the desktop's settings.
        s.hear(Signal::Changed(listed(&[(CLICKING, "")]))).await;
        expected.set(Action::ToggleClicking, Trigger::reported(""));
        assert_eq!(s.take(), [HotkeyEvent::Triggers(expected)]);
        // The session goes on.
        s.press(RECORDING).await;
        assert_eq!(s.take(), [fired(Action::ToggleRecording)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_ends_when_closed_unreadable_or_cut_off_and_says_which() {
    local(async {
        let cases: [(Option<Signal>, &str); 3] = [
            (Some(Signal::Closed), "closed the shortcuts session"),
            (
                Some(Signal::Malformed("Activated".to_owned())),
                "a Activated signal the program cannot read",
            ),
            (None, "the session bus connection ended"),
        ];
        for (signal, why) in cases {
            let s = Served::start().await;
            s.press(CLICKING).await;
            let Served { signals, end, .. } = s;
            match signal {
                Some(signal) => signals.send(signal).unwrap(),
                None => drop(signals),
            }
            let said = tokio::time::timeout(Duration::from_millis(1), end)
                .await
                .expect("the session did not end at once")
                .unwrap();
            assert!(said.contains(why), "{said}");
        }
    })
    .await;
}

fn configure(operation: u64, token: Option<&str>) -> ReaderCommand {
    ReaderCommand::ConfigureShortcuts {
        operation,
        activation_token: token.map(str::to_owned),
    }
}

fn configured(operation: u64, result: Result<(), &str>) -> HotkeyEvent {
    HotkeyEvent::Configured {
        operation,
        result: result.map_err(str::to_owned),
    }
}

#[tokio::test(start_paused = true)]
async fn a_bound_session_opens_the_desktops_settings_and_says_how_it_went() {
    local(async {
        let s = Served::start().await;
        s.commands.send(configure(1, Some("token-1"))).unwrap();
        settle().await;
        assert_eq!(s.take(), [configured(1, Ok(()))]);
        *s.configured.borrow_mut() = Err("refused".to_owned());
        s.commands.send(configure(2, None)).unwrap();
        settle().await;
        assert_eq!(s.take(), [configured(2, Err("refused"))]);
        let tokens = s.tokens.borrow().clone();
        assert_eq!(tokens, [Some("token-1".to_owned()), None]);
        // Hotkeys go on as before.
        s.press(CLICKING).await;
        assert_eq!(s.take(), [fired(Action::ToggleClicking)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn hotkeys_go_on_while_the_desktops_settings_are_being_opened() {
    local(async {
        let s = Served::start().await;
        s.stalls.set(true);
        s.commands.send(configure(1, None)).unwrap();
        settle().await;
        s.press(CLICKING).await;
        assert_eq!(s.take(), [fired(Action::ToggleClicking)]);
        // One at a time: a second request is answered at once.
        s.commands.send(configure(2, None)).unwrap();
        settle().await;
        assert_eq!(s.take(), [configured(2, Err(STILL_OPENING))]);
        // The call ends with the session.
        s.hear(Signal::Closed).await;
        let said = tokio::time::timeout(Duration::from_millis(1), s.end)
            .await
            .expect("the session did not end at once")
            .unwrap();
        assert!(said.contains("closed"), "{said}");
    })
    .await;
}
