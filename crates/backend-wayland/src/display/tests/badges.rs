//! The badges against the fake compositor: what shows for each activity
//! and switch, the corner each sits in, how the countdown gives way to one,
//! and that every way a run ends removes it (O01c).

use auto_clicker_core::config::Corner;
use auto_clicker_core::digits::{CLICKING, RECORDING};

use super::*;
use crate::display::fit::{BADGE, MARGIN};

/// Answers the surface made on the screen `name` after the one `was`, once
/// the client made it, and reads until a buffer is attached to it.
async fn answer_anew(fake: &mut Fake, name: Name, was: u32) {
    let output = fake.ids(name);
    let made = |fake: &Fake| {
        fake.shell
            .on(output)
            .is_some_and(|surface| surface.id != was && surface.commits > 0)
    };
    timeout(BOUND, fake.until(made)).await.unwrap();
    answer(fake, name).await;
}

/// The id of the live surface on the screen `name`.
fn surface_id(fake: &Fake, name: Name) -> u32 {
    fake.shell.on(fake.ids(name)).unwrap().id
}

/// Everything switched on, the badges in `corner`.
fn all_on(corner: Corner) -> OnScreenSettings {
    OnScreenSettings {
        corner,
        ..OnScreenSettings::default()
    }
}

/// What the table expects to show: a countdown's label, a badge's mark,
/// or nothing.
fn shown(view: &View) -> Option<String> {
    wanted(view).map(|wanted| wanted.text().to_owned())
}

#[test]
fn what_shows_for_every_activity_with_each_switch_on_and_off() {
    use Activity::*;
    let on = OnScreenSettings::default();
    let off = |change: fn(&mut OnScreenSettings)| {
        let mut settings = on;
        change(&mut settings);
        settings
    };
    let switches = [
        ("all on", on),
        ("no countdown", off(|s| s.countdown = false)),
        ("no clicking", off(|s| s.clicking = false)),
        ("no recording", off(|s| s.recording = false)),
        ("no playback", off(|s| s.playback = false)),
        (
            "all off",
            off(|s| {
                s.countdown = false;
                s.clicking = false;
                s.recording = false;
                s.playback = false;
            }),
        ),
    ];
    // Each activity, what it shows with everything on, and the switch that
    // turns it off.
    let rows: [(Activity, Option<&str>, &str); 11] = [
        (Idle, None, ""),
        (StartingIn(5), Some("5"), "no countdown"),
        (WaitingForRelease, None, ""),
        (Opening, None, ""),
        (Clicking, Some("pointer"), "no clicking"),
        (RecordingIn(3), Some("3"), "no countdown"),
        (Recording, Some("dot"), "no recording"),
        (PlayingIn(120), Some("2m"), "no countdown"),
        (PlayWaitingForRelease, None, ""),
        (PlayOpening, None, ""),
        (Playing, Some("triangle"), "no playback"),
    ];
    for (activity, with_all, switch) in rows {
        for (name, settings) in switches {
            let expected = with_all.filter(|_| name != switch && name != "all off");
            let got = shown(&view_with(activity, settings));
            assert_eq!(got.as_deref(), expected, "{activity:?} with {name}");
        }
    }
    // The badge's colour is its job's, in the corner the settings name.
    let colour = |activity| match wanted(&view_with(activity, all_on(Corner::BottomRight))) {
        Some(Wanted::Badge { badge, corner }) => (badge.tile, corner),
        other => panic!("{activity:?}: {other:?}"),
    };
    assert_eq!(colour(Clicking), (CLICKING, Corner::BottomRight));
    assert_eq!(colour(Recording), (RECORDING, Corner::BottomRight));
    assert_eq!(colour(Playing), (CLICKING, Corner::BottomRight));
}

#[tokio::test]
async fn each_activity_shows_on_every_screen_as_the_table_says() {
    use Activity::*;
    for (activity, text) in [
        (StartingIn(5), "5"),
        (RecordingIn(3), "3"),
        (PlayingIn(4), "4"),
        (Clicking, "pointer"),
        (Recording, "dot"),
        (Playing, "triangle"),
    ] {
        local(async {
            let harness = Harness::new().await;
            harness.set(activity);
            let mut fake = harness.connected().await;
            answer(&mut fake, A).await;
            answer(&mut fake, B).await;
            harness.shows(&[(A, drawn(text)), (B, drawn(text))]).await;
        })
        .await;
    }
    // The rest show nothing, and open no connection.
    for activity in [
        Idle,
        WaitingForRelease,
        Opening,
        PlayWaitingForRelease,
        PlayOpening,
    ] {
        local(async {
            let harness = Harness::new().await;
            harness.set(activity);
            assert!(!harness.connects().await, "connected for {activity:?}");
        })
        .await;
    }
}

#[tokio::test]
async fn the_badge_sits_in_each_corner_anchored_to_its_two_edges() {
    // Anchors: top 1, bottom 2, left 4, right 8.
    for (corner, anchor) in [
        (Corner::TopLeft, 1 | 4),
        (Corner::TopRight, 1 | 8),
        (Corner::BottomLeft, 2 | 4),
        (Corner::BottomRight, 2 | 8),
    ] {
        local(async {
            let harness = Harness::new().await;
            harness
                .views
                .send_replace(view_with(Activity::Clicking, all_on(corner)));
            let mut fake = harness.connected().await;
            for name in [A, B] {
                answer(&mut fake, name).await;
                let surface = fake.shell.on(fake.ids(name)).unwrap().clone();
                assert_eq!(surface.anchor, Some(anchor), "{corner:?}");
                let margin = MARGIN.cast_signed();
                assert_eq!(surface.margin, Some([margin; 4]), "{corner:?}");
                assert_eq!(surface.size, Some((BADGE, BADGE)), "{corner:?}");
                assert_eq!(last_buffer(&fake, name), (BADGE, BADGE), "{corner:?}");
                // Above everything, ignoring the panels' zones; no input.
                assert_eq!(surface.layer_kind, Some(OVERLAY));
                assert_eq!(surface.namespace.as_deref(), Some(NAMESPACE));
                assert_eq!(surface.exclusive, Some(-1));
                assert_eq!(surface.keyboard, Some(0));
                let region = surface.input.unwrap();
                assert_eq!(fake.shell.regions[&region], Vec::<[u32; 4]>::new());
            }
        })
        .await;
    }
}

#[tokio::test]
async fn the_countdown_gives_way_to_the_badge_on_the_same_connection() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(1));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        harness.shows(&[(A, drawn("1")), (B, drawn("1"))]).await;
        let numbers = [A, B].map(|name| surface_id(&fake, name));
        // The run begins at once, the devices being open.
        harness.set(Activity::Clicking);
        answer_anew(&mut fake, A, numbers[0]).await;
        answer_anew(&mut fake, B, numbers[1]).await;
        harness
            .shows(&[(A, drawn("pointer")), (B, drawn("pointer"))])
            .await;
        assert!(!harness.connects().await, "a second connection");
        for (name, number) in [A, B].into_iter().zip(numbers) {
            // One surface left on each screen: the badge's, anchored; the
            // number's destroyed.
            assert_eq!(live(&fake, name), 1);
            let badge = fake.shell.on(fake.ids(name)).unwrap();
            assert_ne!(badge.id, number);
            assert!(badge.anchor.is_some());
            let destroyed = fake.shell.surfaces.iter().find(|s| s.id == number);
            assert!(destroyed.is_some_and(|s| s.destroyed && s.layer_destroyed));
        }
        // The run ends: the connection closes, the badge with it.
        harness.set(Activity::Idle);
        closes(&mut fake).await;
        harness.shows(&[]).await;
    })
    .await;
}

#[tokio::test]
async fn a_start_that_waits_for_its_devices_shows_the_badge_on_a_new_connection() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::PlayingIn(1));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        harness.set(Activity::PlayOpening);
        closes(&mut fake).await;
        harness.set(Activity::Playing);
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        harness
            .shows(&[(A, drawn("triangle")), (B, drawn("triangle"))])
            .await;
    })
    .await;
}

#[tokio::test]
async fn a_hotkey_start_shows_the_badge_once_its_keys_are_up() {
    for (waiting, running, text) in [
        (Activity::WaitingForRelease, Activity::Clicking, "pointer"),
        (
            Activity::PlayWaitingForRelease,
            Activity::Playing,
            "triangle",
        ),
    ] {
        local(async {
            let harness = Harness::new().await;
            harness.set(waiting);
            assert!(!harness.connects().await, "connected for {waiting:?}");
            harness.set(running);
            let mut fake = harness.connected().await;
            answer(&mut fake, A).await;
            answer(&mut fake, B).await;
            harness.shows(&[(A, drawn(text)), (B, drawn(text))]).await;
        })
        .await;
    }
}

#[tokio::test]
async fn the_badge_goes_at_once_when_the_run_ends() {
    for running in [Activity::Clicking, Activity::Recording, Activity::Playing] {
        local(async {
            let harness = Harness::new().await;
            harness.set(running);
            let mut fake = harness.connected().await;
            answer(&mut fake, A).await;
            // Every end reaches the display as a view without the run.
            harness.set(Activity::Idle);
            closes(&mut fake).await;
            harness.shows(&[]).await;
        })
        .await;
    }
}

#[tokio::test]
async fn a_badge_switched_off_shows_nothing_and_its_switch_on_again_shows_it() {
    for (running, off) in [
        (
            Activity::Clicking,
            OnScreenSettings {
                clicking: false,
                ..OnScreenSettings::default()
            },
        ),
        (
            Activity::Recording,
            OnScreenSettings {
                recording: false,
                ..OnScreenSettings::default()
            },
        ),
        (
            Activity::Playing,
            OnScreenSettings {
                playback: false,
                ..OnScreenSettings::default()
            },
        ),
    ] {
        local(async {
            let harness = Harness::new().await;
            harness.views.send_replace(view_with(running, off));
            assert!(!harness.connects().await, "connected for {running:?}");
            // The next show, after a save.
            harness
                .views
                .send_replace(view_with(Activity::Idle, OnScreenSettings::default()));
            harness.set(running);
            let mut fake = harness.connected().await;
            answer(&mut fake, A).await;
        })
        .await;
    }
}

#[tokio::test]
async fn a_screen_that_failed_during_the_countdown_is_tried_again_for_the_badge() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(2));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        // B never answers the number: late, and not tried again on a tick.
        harness
            .shows(&[(A, drawn("2")), (B, Showing::Missing(Missing::Late))])
            .await;
        harness.set(Activity::StartingIn(1));
        harness
            .shows(&[(A, drawn("1")), (B, Showing::Missing(Missing::Late))])
            .await;
        // The badge is the next show: B is asked again.
        let number = surface_id(&fake, A);
        harness.set(Activity::Clicking);
        answer_anew(&mut fake, A, number).await;
        answer(&mut fake, B).await;
        harness
            .shows(&[(A, drawn("pointer")), (B, drawn("pointer"))])
            .await;
    })
    .await;
}

#[tokio::test]
async fn a_screen_in_the_middle_of_an_update_takes_the_badge_only_once_it_ends() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(2));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        let numbers = [A, B].map(|name| surface_id(&fake, name));
        // B gives a new size and has not said its update ended; A's next
        // configure, acked once read, shows the client read B's size first.
        let Ids { xdg, .. } = fake.ids(B);
        let size = [BELOW.width, BELOW.height].map(i32::cast_unsigned);
        let on_a = fake.shell.on(fake.ids(A)).unwrap().clone();
        let resizing = Message {
            object: xdg,
            opcode: 1,
            words: size.to_vec(),
        };
        fake.send(vec![resizing, on_a.configure(8)]).await;
        let output = fake.ids(A);
        let read = |fake: &Fake| fake.shell.on(output).unwrap().acked.contains(&8);
        timeout(BOUND, fake.until(read)).await.unwrap();
        harness.set(Activity::Clicking);
        answer_anew(&mut fake, A, numbers[0]).await;
        // B's number goes, and nothing is drawn there at the number's size.
        gone(&mut fake, B).await;
        harness.shows(&[(A, drawn("pointer"))]).await;
        fake.screen(B, BELOW).await;
        answer_anew(&mut fake, B, numbers[1]).await;
        assert_eq!(last_buffer(&fake, B), (BADGE, BADGE));
        harness
            .shows(&[(A, drawn("pointer")), (B, drawn("pointer"))])
            .await;
    })
    .await;
}

#[tokio::test]
async fn a_connection_lost_during_the_countdown_comes_back_for_the_badge() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(3));
        drop(harness.connected().await);
        // A tick is no new show.
        harness.set(Activity::StartingIn(2));
        assert!(!harness.connects().await, "tried again on a tick");
        harness.set(Activity::Recording);
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        // Lost again while the badge shows: it waits for the next show.
        drop(fake);
        harness.set(Activity::Recording);
        assert!(
            !harness.connects().await,
            "tried again while the same badge shows"
        );
    })
    .await;
}

#[tokio::test]
async fn the_badge_is_drawn_once_and_again_only_at_a_new_scale() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::Recording);
        let mut fake = harness.connected().await;
        answer(&mut fake, B).await;
        let output = fake.ids(B);
        let surface = fake.shell.on(output).unwrap().clone();
        assert_eq!(surface.attached.len(), 1);
        let attached = |fake: &Fake| fake.shell.on(output).unwrap().attached.len();
        // The view changes, the badge does not: nothing is drawn.
        harness.views.send_replace(view(Activity::Recording, false));
        harness.set(Activity::Recording);
        let more = timeout(AT_ONCE, fake.until(|fake| attached(fake) > 1)).await;
        assert!(more.is_err(), "drawn again for the same badge");
        // At 1.25 it is drawn again, 50 px, shown at 40.
        fake.send(vec![surface.preferred(150)]).await;
        timeout(BOUND, fake.until(|fake| attached(fake) > 1))
            .await
            .unwrap();
        assert_eq!(last_buffer(&fake, B), (50, 50));
        let surface = fake.shell.on(output).unwrap();
        assert_eq!(surface.destination, Some((40, 40)));
        let more = timeout(AT_ONCE, fake.until(|fake| attached(fake) > 2)).await;
        assert!(more.is_err(), "drawn again at the same scale");
    })
    .await;
}

#[tokio::test]
async fn a_screen_that_changes_size_keeps_its_badge() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::Clicking);
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        let output = fake.ids(A);
        let before = fake.shell.on(output).unwrap().id;
        fake.screen(A, BELOW).await;
        let anew = |fake: &Fake| fake.shell.on(output).is_none_or(|now| now.id != before);
        let made = timeout(AT_ONCE, fake.until(anew)).await;
        assert!(made.is_err(), "a new surface for the same badge");
        assert_eq!(live(&fake, A), 1);
        harness
            .shows(&[(A, drawn("pointer")), (B, Showing::Asked)])
            .await;
    })
    .await;
}

#[tokio::test]
async fn an_absurd_scale_leaves_that_screen_without_a_badge_and_the_other_with_its() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::Playing);
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        let surface = fake.shell.on(fake.ids(A)).unwrap().clone();
        // 40 px at 20 times is past what a badge's buffer may hold.
        fake.send(vec![surface.preferred(WHOLE_TIMES_20)]).await;
        harness
            .shows(&[
                (A, Showing::Missing(Missing::Buffer)),
                (B, drawn("triangle")),
            ])
            .await;
        gone(&mut fake, A).await;
    })
    .await;
}

/// A scale of 20, in 120ths.
const WHOLE_TIMES_20: u32 = 120 * 20;
