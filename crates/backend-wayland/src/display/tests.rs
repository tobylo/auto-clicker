//! The on-screen display against a fake compositor at a path of its own:
//! when its connection is made and closed, what each screen asks for and
//! shows, and why a screen shows nothing.

use std::num::NonZeroU32;
use std::sync::Arc;

use auto_clicker_core::config::Config;
use auto_clicker_core::controller::{Activity, Hotkey, LayoutSignal, Routes};
use tokio::task::{JoinHandle, LocalSet};
use tokio::time::timeout;

use super::*;
use crate::client::fake::{
    A, B, BELOW, BOUND, C, Display, Fake, Ids, LEFT, Message, Name, OVERLAY, Offer, Place, RIGHT,
    SHELL, release, removed,
};
use crate::display::fit::Missing;

mod ends;

/// Well under [`SETUP`] and [`ANSWER`]: a stop that waits either out
/// fails.
const AT_ONCE: Duration = Duration::from_millis(300);

/// The view while `activity` runs, the countdown on the screen switched on
/// or off.
fn view(activity: Activity, countdown: bool) -> View {
    let config = Config::default();
    let mut on_screen = config.on_screen;
    on_screen.countdown = countdown;
    View {
        activity,
        routes: Routes::default(),
        settings: config.clicker,
        hotkey: Hotkey::Bound(config.hotkeys.toggle_clicking),
        record_hotkey: Hotkey::Bound(config.hotkeys.toggle_recording),
        no_recording: None,
        play_hotkey: Hotkey::Bound(config.hotkeys.toggle_playback),
        playback: config.playback,
        on_screen,
        recordings: Arc::from([]),
        current: None,
        playing: None,
        notice: None,
        problems: Vec::new(),
    }
}

/// A display at its own path that offered the on-screen display's globals
/// at start (or not), the display's watcher on it, and the view it reads.
struct Harness {
    display: Display,
    on_screen: OnScreen,
    wayland: Wayland,
    views: watch::Sender<View>,
    /// The layout connection's far end, kept open.
    layout: Fake,
    _watch: JoinHandle<()>,
}

impl Harness {
    async fn new() -> Harness {
        Harness::offering(true).await
    }

    /// The same, the display's globals offered at start or not.
    async fn offering(offered: bool) -> Harness {
        Harness::listened_by(offered, NonZeroU32::new(std::process::id())).await
    }

    /// The same, the display's listener found at start being `compositor`.
    async fn listened_by(offered: bool, compositor: Option<NonZeroU32>) -> Harness {
        let display = Display::new();
        let (link, mut fake) = Fake::pair();
        fake.display = offered;
        let (started, ()) = timeout(BOUND, async {
            tokio::join!(Link::start(link), fake.start_with_two())
        })
        .await
        .unwrap();
        let (link, first) = started.unwrap();
        let wayland = Wayland::of(compositor, Some(display.path.clone()), first, Ok(link));
        let on_screen = OnScreen::of(&wayland);
        let (views, shown) = watch::channel(view(Activity::Idle, true));
        let watch = on_screen.watch(shown);
        Harness {
            display,
            on_screen,
            wayland,
            views,
            layout: fake,
            _watch: tokio::task::spawn_local(watch),
        }
    }

    fn set(&self, activity: Activity) {
        self.views.send_replace(view(activity, true));
    }

    /// The display's connection, accepted and set up: two screens side by
    /// side, and the display's globals.
    async fn connected(&self) -> Fake {
        let mut fake = timeout(BOUND, self.display.accept()).await.unwrap();
        fake.display = true;
        timeout(BOUND, fake.start_with_two()).await.unwrap();
        fake
    }

    /// Whether a connection is made within [`AT_ONCE`].
    async fn connects(&self) -> bool {
        timeout(AT_ONCE, self.display.accept()).await.is_ok()
    }

    /// Waits until the display holds `screens` as each one shows.
    async fn shows(&self, screens: &[(Name, Showing)]) {
        let expected: BTreeMap<u32, Showing> = screens
            .iter()
            .map(|(Name(name), showing)| (*name, showing.clone()))
            .collect();
        let waited = timeout(BOUND, async {
            while self.on_screen.screens() != expected {
                yield_now().await;
            }
        })
        .await;
        assert!(waited.is_ok(), "{:?}", self.on_screen.screens());
    }
}

/// Runs `test` on a local set, as the daemon's thread runs the display.
async fn local(test: impl std::future::Future<Output = ()>) {
    LocalSet::new().run_until(test).await;
}

/// Reads until the screen `name` has a layer surface, answers it, and
/// reads until a buffer is attached to it.
async fn answer(fake: &mut Fake, name: Name) {
    let output = fake.ids(name);
    timeout(BOUND, fake.until(|fake| asked(fake, output)))
        .await
        .unwrap();
    let surface = fake.shell.on(output).unwrap().clone();
    fake.send(vec![surface.configure(7)]).await;
    let attached = surface.attached.len();
    timeout(
        BOUND,
        fake.until(|fake| fake.shell.on(output).unwrap().attached.len() > attached),
    )
    .await
    .unwrap();
}

/// Whether the surface on `output` was asked for: made and committed.
fn asked(fake: &Fake, output: Ids) -> bool {
    fake.shell
        .on(output)
        .is_some_and(|surface| surface.commits > 0)
}

/// The last buffer attached to the screen `name`'s surface: its size.
fn last_buffer(fake: &Fake, name: Name) -> (u32, u32) {
    let surface = fake.shell.on(fake.ids(name)).unwrap();
    let buffer = fake.shell.buffers[surface.attached.last().unwrap()];
    assert_eq!(buffer.stride, buffer.width * 4);
    (buffer.width, buffer.height)
}

/// Reads until the screen `name`'s surface has had `count` buffers
/// attached, releases each but the last, and reads until the client
/// destroyed them.
async fn release_shown(fake: &mut Fake, name: Name, count: usize) {
    let output = fake.ids(name);
    let attached = |fake: &Fake| fake.shell.on(output).unwrap().attached.clone();
    timeout(BOUND, fake.until(|fake| attached(fake).len() >= count))
        .await
        .unwrap();
    let attached = attached(fake);
    let (_, shown) = attached.split_last().unwrap();
    fake.send(shown.iter().map(|&buffer| release(buffer)).collect())
        .await;
    let gone = |fake: &Fake| {
        shown
            .iter()
            .all(|buffer| fake.shell.destroyed.contains(buffer))
    };
    timeout(BOUND, fake.until(gone)).await.unwrap();
}

/// Reads until the screen `name` has no surface left: each made on it
/// was destroyed.
async fn gone(fake: &mut Fake, name: Name) {
    timeout(BOUND, fake.until(|fake| live(fake, name) == 0))
        .await
        .unwrap();
}

/// How many surfaces on the screen `name` were not destroyed.
fn live(fake: &Fake, name: Name) -> usize {
    let Ids { output, .. } = fake.ids(name);
    let surfaces = fake.shell.surfaces.iter();
    surfaces
        .filter(|surface| surface.output == Some(output) && !surface.destroyed)
        .count()
}

/// Waits until the client closed `fake`'s connection.
async fn closes(fake: &mut Fake) {
    let closed = timeout(AT_ONCE, fake.closed()).await;
    assert_eq!(closed.ok(), Some(true), "the connection stayed open");
}

fn drawn(text: &str) -> Showing {
    Showing::Drawn(text.to_owned())
}

#[tokio::test]
async fn nothing_of_the_display_exists_until_a_countdown_from_the_tray() {
    local(async {
        let harness = Harness::new().await;
        assert!(!harness.connects().await, "connected while idle");
        // A hotkey's start waits for its keys and does not count down.
        for activity in [
            Activity::WaitingForRelease,
            Activity::Opening,
            Activity::Clicking,
        ] {
            harness.set(activity);
            assert!(!harness.connects().await, "connected for {activity:?}");
        }
        harness.set(Activity::StartingIn(5));
        assert!(harness.connects().await);
    })
    .await;
}

#[tokio::test]
async fn a_tray_countdown_shows_on_every_screen_follows_the_seconds_and_goes() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        // A quarter of 1280 is 320; "5" is 176 wide, padded by 80.
        assert_eq!(last_buffer(&fake, A), (336, 480));
        answer(&mut fake, B).await;
        assert_eq!(last_buffer(&fake, B), (378, 540));
        harness.shows(&[(A, drawn("5")), (B, drawn("5"))]).await;
        harness.set(Activity::StartingIn(4));
        harness.shows(&[(A, drawn("4")), (B, drawn("4"))]).await;
        // The compositor is done with the first buffers, as it is once the
        // next is shown.
        release_shown(&mut fake, A, 2).await;
        release_shown(&mut fake, B, 2).await;
        // Ten is wider: each surface is asked for its new size first.
        let sizes = [A, B].map(|name| fake.shell.on(fake.ids(name)).unwrap().size);
        harness.set(Activity::RecordingIn(10));
        harness
            .shows(&[(A, Showing::Asked), (B, Showing::Asked)])
            .await;
        for (name, was) in [A, B].into_iter().zip(sizes) {
            let output = fake.ids(name);
            let asked = |fake: &Fake| fake.shell.on(output).unwrap().size != was;
            timeout(BOUND, fake.until(asked)).await.unwrap();
        }
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        harness.shows(&[(A, drawn("10")), (B, drawn("10"))]).await;
        // The run begins: the connection closes, and every surface with it.
        harness.set(Activity::Recording);
        closes(&mut fake).await;
        harness.shows(&[]).await;
    })
    .await;
}

#[tokio::test]
async fn each_surface_is_on_the_overlay_layer_and_takes_no_input() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::PlayingIn(3));
        let mut fake = harness.connected().await;
        for name in [A, B] {
            let output = fake.ids(name);
            timeout(BOUND, fake.until(|fake| asked(fake, output)))
                .await
                .unwrap();
            let surface = fake.shell.on(output).unwrap().clone();
            assert_eq!(surface.layer_kind, Some(OVERLAY));
            assert_eq!(surface.namespace.as_deref(), Some(NAMESPACE));
            // No keyboard, no exclusive zone, centred: no anchor.
            assert_eq!(surface.keyboard, Some(0));
            assert_eq!(surface.exclusive, Some(-1));
            assert_eq!(surface.anchor, None);
            // An input region set, and empty.
            let region = surface.input.unwrap();
            assert_ne!(region, 0);
            assert_eq!(fake.shell.regions[&region], Vec::<[u32; 4]>::new());
            assert!(surface.viewport.is_some() && surface.fraction.is_some());
        }
    })
    .await;
}

#[tokio::test]
async fn a_cancelled_countdown_removes_the_number_at_once() {
    for cancel in [Activity::Idle, Activity::Clicking] {
        local(async {
            let harness = Harness::new().await;
            harness.set(Activity::StartingIn(5));
            let mut fake = harness.connected().await;
            answer(&mut fake, A).await;
            harness.set(cancel);
            closes(&mut fake).await;
        })
        .await;
    }
}

#[tokio::test]
async fn an_answer_after_the_countdown_ended_shows_nothing() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        let output = fake.ids(A);
        timeout(BOUND, fake.until(|fake| asked(fake, output)))
            .await
            .unwrap();
        let surface = fake.shell.on(output).unwrap().clone();
        harness.set(Activity::Clicking);
        // The answer comes after: the connection is closed, nothing drawn.
        let _ = fake.stream.try_write(&surface.configure(1).bytes());
        closes(&mut fake).await;
        assert!(fake.shell.surfaces.iter().all(|s| s.attached.is_empty()));
    })
    .await;
}

#[tokio::test]
async fn a_display_that_never_answers_its_setup_is_left_at_once_when_the_countdown_ends() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        // The client's first round trip, never answered.
        timeout(BOUND, fake.until_sync()).await.unwrap();
        harness.set(Activity::Clicking);
        closes(&mut fake).await;
    })
    .await;
}

#[tokio::test]
async fn shutdown_closes_the_displays_connection() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        harness._watch.abort();
        closes(&mut fake).await;
    })
    .await;
}

#[tokio::test]
async fn switched_off_the_countdown_shows_nothing() {
    local(async {
        let harness = Harness::new().await;
        harness
            .views
            .send_replace(view(Activity::StartingIn(5), false));
        assert!(!harness.connects().await, "connected while switched off");
        harness
            .views
            .send_replace(view(Activity::StartingIn(4), true));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        harness
            .views
            .send_replace(view(Activity::StartingIn(3), false));
        closes(&mut fake).await;
    })
    .await;
}

#[tokio::test]
async fn the_buffer_follows_the_screens_fractional_scale_at_its_logical_size() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, B).await;
        let output = fake.ids(B);
        let surface = fake.shell.on(output).unwrap().clone();
        assert_eq!(surface.destination, Some((378, 540)));
        let drawn = surface.attached.len();
        fake.send(vec![surface.preferred(150)]).await;
        timeout(
            BOUND,
            fake.until(|fake| fake.shell.on(output).unwrap().attached.len() > drawn),
        )
        .await
        .unwrap();
        // 378 x 540 at 1.25, shown at 378 x 540.
        assert_eq!(last_buffer(&fake, B), (473, 675));
        let surface = fake.shell.on(output).unwrap();
        assert_eq!(surface.destination, Some((378, 540)));
    })
    .await;
}

#[tokio::test]
async fn a_screen_added_while_it_shows_gets_it_and_one_removed_loses_it() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        harness.shows(&[(A, drawn("5")), (B, drawn("5"))]).await;
        fake.offer(Offer::Output(C)).await;
        timeout(BOUND, fake.until(|fake| fake.outputs.len() == 3))
            .await
            .unwrap();
        fake.screen(C, BELOW).await;
        answer(&mut fake, C).await;
        harness
            .shows(&[(A, drawn("5")), (B, drawn("5")), (C, drawn("5"))])
            .await;
        let output = fake.ids(B);
        fake.send(vec![removed(B)]).await;
        timeout(BOUND, fake.until(|fake| fake.shell.on(output).is_none()))
            .await
            .unwrap();
        harness.shows(&[(A, drawn("5")), (C, drawn("5"))]).await;
    })
    .await;
}

#[tokio::test]
async fn a_closed_surface_or_a_late_answer_on_one_screen_leaves_the_other_showing() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        // B is never answered: past its deadline it shows nothing, and its
        // surface goes.
        harness
            .shows(&[(A, drawn("5")), (B, Showing::Missing(Missing::Late))])
            .await;
        gone(&mut fake, B).await;
        let surface = fake.shell.on(fake.ids(A)).unwrap().clone();
        fake.send(vec![surface.closed()]).await;
        harness
            .shows(&[
                (A, Showing::Missing(Missing::Closed)),
                (B, Showing::Missing(Missing::Late)),
            ])
            .await;
        gone(&mut fake, A).await;
        // Its buffer goes with it.
        let freed = |fake: &Fake| {
            let buffers = surface.attached.iter();
            buffers
                .clone()
                .all(|buffer| fake.shell.destroyed.contains(buffer))
        };
        timeout(BOUND, fake.until(freed)).await.unwrap();
        // A tick of the seconds tries neither again.
        harness.set(Activity::StartingIn(4));
        assert!(!harness.connects().await);
        harness
            .shows(&[
                (A, Showing::Missing(Missing::Closed)),
                (B, Showing::Missing(Missing::Late)),
            ])
            .await;
    })
    .await;
}

/// A fake that offers `places` as its outputs, named from 20, and the
/// display's globals; each output's update sent once it is bound.
async fn start_with(fake: &mut Fake, places: &[Place]) -> Vec<Name> {
    let names: Vec<Name> = (0..places.len())
        .map(|index| Name(20 + u32::try_from(index).unwrap()))
        .collect();
    let callback = fake.until_sync().await;
    fake.offer(Offer::Manager).await;
    for &name in &names {
        fake.offer(Offer::Output(name)).await;
    }
    for offer in Offer::display() {
        fake.offer(offer).await;
    }
    fake.answer(callback).await;
    let callback = fake.until_sync().await;
    for (&name, &place) in names.iter().zip(places) {
        fake.screen(name, place).await;
    }
    fake.answer(callback).await;
    fake.round_trip().await;
    names
}

#[tokio::test]
async fn screens_past_the_bounds_show_nothing_and_keep_the_reason() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        // Seventeen screens: the last is past what a layout may hold. The
        // first is too tall for its countdown's surface.
        let mut places = vec![RIGHT; 17];
        places[0] = Place {
            height: 20_000,
            ..RIGHT
        };
        let names = timeout(BOUND, start_with(&mut fake, &places))
            .await
            .unwrap();
        let tall = names[0];
        let last = names[16];
        let shown: Vec<(Name, Showing)> = names
            .iter()
            .map(|&name| {
                let showing = if name == tall {
                    Showing::Missing(Missing::Size)
                } else if name == last {
                    Showing::Missing(Missing::TooMany)
                } else {
                    Showing::Asked
                };
                (name, showing)
            })
            .collect();
        harness.shows(&shown).await;
        // A scale that overflows the buffer's size.
        let second = names[1];
        let output = fake.ids(second);
        timeout(BOUND, fake.until(|fake| asked(fake, output)))
            .await
            .unwrap();
        let surface = fake.shell.on(output).unwrap().clone();
        fake.send(vec![surface.preferred(u32::MAX), surface.configure(3)])
            .await;
        let waited = timeout(BOUND, async {
            while harness.on_screen.screens().get(&second.0)
                != Some(&Showing::Missing(Missing::Buffer))
            {
                yield_now().await;
            }
        })
        .await;
        assert!(waited.is_ok(), "{:?}", harness.on_screen.screens());
    })
    .await;
}

#[tokio::test]
async fn a_lost_connection_comes_back_with_the_layouts_signal_and_not_before() {
    local(async {
        let mut harness = Harness::new().await;
        let changed = LayoutSignal::default();
        let layout = harness.wayland.watch(changed, LayoutSignal::default());
        let _layout = tokio::task::spawn_local(layout);
        harness.set(Activity::StartingIn(5));
        let fake = harness.connected().await;
        drop(fake);
        // Ticks of the seconds bring nothing back.
        for seconds in [4, 3] {
            harness.set(Activity::StartingIn(seconds));
            assert!(!harness.connects().await, "connected again at {seconds}");
        }
        // The layout changes: a screen goes.
        harness.layout.send(vec![removed(B)]).await;
        timeout(BOUND, harness.layout.round_trip()).await.unwrap();
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        fake.display = true;
        timeout(BOUND, fake.start_with_two()).await.unwrap();
        answer(&mut fake, A).await;
        harness.shows(&[(A, drawn("3")), (B, Showing::Asked)]).await;
    })
    .await;
}

#[tokio::test]
async fn a_protocol_error_on_the_displays_connection_leaves_the_layout_known() {
    local(async {
        let harness = Harness::new().await;
        let before = harness.wayland.layout();
        assert!(before.is_ok());
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        let output = fake.ids(A);
        timeout(BOUND, fake.until(|fake| asked(fake, output)))
            .await
            .unwrap();
        let surface = fake.shell.on(output).unwrap().clone();
        fake.send(vec![surface.error()]).await;
        closes(&mut fake).await;
        harness.shows(&[]).await;
        assert_eq!(harness.wayland.layout(), before);
        // Nothing tries again until the screens change.
        harness.set(Activity::StartingIn(4));
        assert!(!harness.connects().await);
    })
    .await;
}

#[tokio::test]
async fn a_display_without_layer_shell_cannot_show_and_is_never_asked() {
    local(async {
        let harness = Harness::offering(false).await;
        assert_eq!(harness.on_screen.found(), Err(NoOnScreen::NoLayerShell));
        harness.set(Activity::StartingIn(5));
        assert!(!harness.connects().await);
    })
    .await;
}

#[test]
fn why_the_display_cannot_show_is_what_the_layouts_connection_found() {
    for (why, said) in [
        (NoScreens::NoDisplay, NoOnScreen::NoDisplay),
        (NoScreens::CannotConnect, NoOnScreen::CannotConnect),
        (NoScreens::NoXdgOutput, NoOnScreen::NoXdgOutput),
        (NoScreens::Lost, NoOnScreen::CannotConnect),
        (NoScreens::ProtocolError, NoOnScreen::CannotConnect),
    ] {
        assert_eq!(found(Err(why)), Err(said), "{why:?}");
        assert_eq!(OnScreen::of(&Wayland::none(why)).found(), Err(said));
    }
}

#[tokio::test]
async fn a_layout_connection_that_offers_it_all_can_show() {
    local(async {
        let harness = Harness::new().await;
        assert_eq!(harness.on_screen.found(), Ok(()));
    })
    .await;
}

#[tokio::test]
async fn a_global_withdrawn_while_the_first_globals_are_heard_is_not_used() {
    let (link, mut fake) = Fake::pair();
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), async {
            let callback = fake.until_sync().await;
            fake.offer(Offer::Manager).await;
            for offer in Offer::display() {
                fake.offer(offer).await;
            }
            fake.send(vec![removed(SHELL)]).await;
            fake.answer(callback).await;
            fake.round_trip().await;
        })
    })
    .await
    .unwrap();
    let (link, _) = started.unwrap();
    assert_eq!(found(Ok(&link)), Err(NoOnScreen::NoLayerShell));
}

#[tokio::test]
async fn an_old_compositor_cannot_show() {
    let (link, mut fake) = Fake::pair();
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), async {
            let callback = fake.until_sync().await;
            fake.offer(Offer::Manager).await;
            for offer in [Offer::Compositor(3), Offer::Shm, Offer::LayerShell] {
                fake.offer(offer).await;
            }
            fake.answer(callback).await;
            fake.round_trip().await;
        })
    })
    .await
    .unwrap();
    let (link, _) = started.unwrap();
    assert_eq!(found(Ok(&link)), Err(NoOnScreen::NoLayerShell));
}

#[tokio::test]
async fn a_screen_change_heard_while_it_showed_does_not_bring_a_lost_connection_back() {
    local(async {
        let mut harness = Harness::new().await;
        let layout = harness
            .wayland
            .watch(LayoutSignal::default(), LayoutSignal::default());
        let _layout = tokio::task::spawn_local(layout);
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        // The layout changes while the number shows.
        let before = harness.wayland.layout();
        harness.layout.send(vec![removed(B)]).await;
        timeout(BOUND, harness.layout.round_trip()).await.unwrap();
        let heard = timeout(BOUND, async {
            while harness.wayland.layout() == before {
                yield_now().await;
            }
        })
        .await;
        assert!(heard.is_ok(), "the layout did not change");
        drop(fake);
        harness.set(Activity::StartingIn(4));
        assert!(
            !harness.connects().await,
            "connected again on an old change"
        );
    })
    .await;
}

#[tokio::test]
async fn the_layouts_connection_made_again_says_anew_whether_the_display_can_show() {
    local(async {
        let harness = Harness::offering(false).await;
        assert_eq!(harness.on_screen.found(), Err(NoOnScreen::NoLayerShell));
        let (changed, again) = (LayoutSignal::default(), LayoutSignal::default());
        let layout = harness.wayland.watch(changed, again.clone());
        let _layout = tokio::task::spawn_local(layout);
        // A new KWin: the layout's connection is made again, to a display
        // that offers layer-shell now.
        again.changed();
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        fake.display = true;
        timeout(BOUND, fake.start_with_two()).await.unwrap();
        let found = timeout(BOUND, async {
            while harness.on_screen.found().is_err() {
                yield_now().await;
            }
        })
        .await;
        assert!(found.is_ok(), "{:?}", harness.on_screen.found());
    })
    .await;
}

#[tokio::test]
async fn a_display_another_process_listens_on_is_refused_at_once_and_not_retried() {
    local(async {
        let harness = Harness::listened_by(true, NonZeroU32::new(1)).await;
        harness.set(Activity::StartingIn(5));
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        closes(&mut fake).await;
        harness.shows(&[]).await;
        harness.set(Activity::StartingIn(4));
        assert!(!harness.connects().await, "tried again on a tick");
    })
    .await;
}

#[tokio::test]
async fn a_buffer_the_compositor_released_is_destroyed() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        let surface = fake.shell.on(fake.ids(A)).unwrap().clone();
        let buffer = *surface.attached.last().unwrap();
        assert!(fake.shell.destroyed.is_empty());
        fake.send(vec![release(buffer)]).await;
        let destroyed = timeout(BOUND, fake.until(|fake| !fake.shell.destroyed.is_empty())).await;
        assert!(destroyed.is_ok());
        assert_eq!(fake.shell.destroyed, [buffer]);
    })
    .await;
}

#[tokio::test]
async fn one_screen_that_never_ends_its_update_leaves_the_others_showing() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        let callback = fake.until_sync().await;
        for offer in [Offer::Manager, Offer::Output(A), Offer::Output(B)] {
            fake.offer(offer).await;
        }
        for offer in Offer::display() {
            fake.offer(offer).await;
        }
        fake.answer(callback).await;
        let callback = fake.until_sync().await;
        fake.screen(A, LEFT).await;
        // B gives its size and never says its update ended.
        let Ids { xdg, .. } = fake.ids(B);
        let size = [RIGHT.width, RIGHT.height].map(i32::cast_unsigned);
        let words = size.to_vec();
        fake.send(vec![Message {
            object: xdg,
            opcode: 1,
            words,
        }])
        .await;
        fake.answer(callback).await;
        answer(&mut fake, A).await;
        harness.shows(&[(A, drawn("5"))]).await;
    })
    .await;
}

#[tokio::test]
async fn an_answer_read_after_its_deadline_shows_nothing() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        let output = fake.ids(B);
        timeout(BOUND, fake.until(|fake| asked(fake, output)))
            .await
            .unwrap();
        let surface = fake.shell.on(output).unwrap().clone();
        fake.send(vec![surface.configure(9)]).await;
        // The thread is held past the deadline, so the display wakes to
        // the answer and the deadline at once.
        std::thread::sleep(ANSWER + Duration::from_millis(200));
        harness
            .shows(&[(A, drawn("5")), (B, Showing::Missing(Missing::Late))])
            .await;
    })
    .await;
}

#[tokio::test]
async fn a_screen_holds_two_buffers_until_the_compositor_releases_one() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        answer(&mut fake, B).await;
        harness.set(Activity::StartingIn(4));
        harness.shows(&[(A, drawn("4")), (B, drawn("4"))]).await;
        // Neither screen's buffers are released: the next second waits
        // for one, on B until its release and on A for longer.
        harness.set(Activity::StartingIn(3));
        release_shown(&mut fake, B, 2).await;
        harness.shows(&[(A, drawn("4")), (B, drawn("3"))]).await;
        let attached = |fake: &Fake| fake.shell.on(fake.ids(A)).unwrap().attached.len();
        assert_eq!(attached(&fake), 2);
        release_shown(&mut fake, A, 2).await;
        harness.shows(&[(A, drawn("3")), (B, drawn("3"))]).await;
        timeout(BOUND, fake.until(|fake| attached(fake) == 3))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_screen_that_changes_size_is_tried_again() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        let tall = Place {
            height: 20_000,
            ..RIGHT
        };
        let names = timeout(BOUND, start_with(&mut fake, &[tall, LEFT]))
            .await
            .unwrap();
        let shown = [
            (names[0], Showing::Missing(Missing::Size)),
            (names[1], Showing::Asked),
        ];
        harness.shows(&shown).await;
        fake.screen(names[0], RIGHT).await;
        harness
            .shows(&[(names[0], Showing::Asked), (names[1], Showing::Asked)])
            .await;
        // A screen that shows and changes size keeps its one surface, asked
        // for the new size.
        answer(&mut fake, names[1]).await;
        let before = fake.shell.on(fake.ids(names[1])).unwrap().size;
        fake.screen(names[1], BELOW).await;
        harness
            .shows(&[(names[0], Showing::Asked), (names[1], Showing::Asked)])
            .await;
        let output = fake.ids(names[1]);
        let resized = |fake: &Fake| fake.shell.on(output).unwrap().size != before;
        timeout(BOUND, fake.until(resized)).await.unwrap();
        assert_eq!(live(&fake, names[1]), 1);
    })
    .await;
}

#[tokio::test]
async fn a_screen_that_showed_nothing_is_tried_again_when_a_screen_is_added() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        // B is never answered.
        harness
            .shows(&[(A, drawn("5")), (B, Showing::Missing(Missing::Late))])
            .await;
        fake.offer(Offer::Output(C)).await;
        timeout(BOUND, fake.until(|fake| fake.outputs.len() == 3))
            .await
            .unwrap();
        fake.screen(C, BELOW).await;
        harness
            .shows(&[(A, drawn("5")), (B, Showing::Asked), (C, Showing::Asked)])
            .await;
        // Late again, B is tried again when a screen goes too.
        answer(&mut fake, C).await;
        harness
            .shows(&[
                (A, drawn("5")),
                (B, Showing::Missing(Missing::Late)),
                (C, drawn("5")),
            ])
            .await;
        fake.send(vec![removed(C)]).await;
        harness.shows(&[(A, drawn("5")), (B, Showing::Asked)]).await;
    })
    .await;
}

#[tokio::test]
async fn a_new_size_asked_before_the_first_is_answered_takes_a_new_surface() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        let outputs = [A, B].map(|name| fake.ids(name));
        let all_asked = move |fake: &Fake| outputs.iter().all(|&output| asked(fake, output));
        timeout(BOUND, fake.until(all_asked)).await.unwrap();
        let first = fake.shell.on(outputs[0]).unwrap().clone();
        // Ten is wider: asked before the first is answered, then wider
        // again, each on a surface of its own.
        for seconds in [10, 100] {
            let shown = outputs.map(|output| fake.shell.on(output).unwrap().id);
            harness.set(Activity::StartingIn(seconds));
            let anew = move |fake: &Fake| {
                let surface = |output| fake.shell.on(output).map(|s| (s.id, s.commits));
                (0..2).all(|i| {
                    surface(outputs[i]).is_some_and(|(id, commits)| id != shown[i] && commits > 0)
                })
            };
            timeout(BOUND, fake.until(anew)).await.unwrap();
        }
        assert_eq!((live(&fake, A), live(&fake, B)), (1, 1));
        // The first surface's answer comes late: it answers nothing. B's,
        // sent after it, shows that it was read.
        fake.send(vec![first.configure(3)]).await;
        answer(&mut fake, B).await;
        harness
            .shows(&[(A, Showing::Asked), (B, drawn("2m"))])
            .await;
        answer(&mut fake, A).await;
        harness.shows(&[(A, drawn("2m")), (B, drawn("2m"))]).await;
    })
    .await;
}
