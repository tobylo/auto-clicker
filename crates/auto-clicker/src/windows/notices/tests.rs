use std::future::pending;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc as std_mpsc};
use std::thread;
use std::time::Instant;

use auto_clicker_backend_windows::MessageThread;
use auto_clicker_core::clicker::{ClickerSettings, Mode};
use auto_clicker_core::config::Config;
use auto_clicker_core::controller::{
    Activity, Controller, HotkeyReader, InputLifetime, Links, OpenInput, Platform, ReadInput, View,
};
use auto_clicker_core::input::fake::{Call, FakeBackend, FakeLog};
use auto_clicker_core::input::{Input, PressState};
use auto_clicker_core::recording::LocalTime;
use tokio::sync::watch;

use super::*;

const UNLOCKED: SessionState = SessionState {
    locked: Some(false),
    active: true,
};

fn unknown(why: &str) -> Told {
    Told::Session(Some(Blocked::Unknown(why.to_owned().into())))
}

fn lock() -> WindowEvent {
    WindowEvent::Session(SessionChange::Lock)
}

fn unlock() -> WindowEvent {
    WindowEvent::Session(SessionChange::Unlock)
}

fn connect() -> WindowEvent {
    WindowEvent::Session(SessionChange::Connect)
}

fn disconnect() -> WindowEvent {
    WindowEvent::Session(SessionChange::Disconnect)
}

/// Each event in turn, and what the controller is told after it.
fn told(events: Vec<WindowEvent>) -> Vec<Told> {
    let mut session = Session::default();
    events.iter().map(|event| session.apply(event)).collect()
}

#[test]
fn the_first_read_says_whether_clicking_may_run() {
    assert_eq!(
        told(vec![WindowEvent::SessionRead(Ok(UNLOCKED))]),
        [Told::Session(None)]
    );
    let locked = SessionState {
        locked: Some(true),
        active: true,
    };
    assert_eq!(
        told(vec![WindowEvent::SessionRead(Ok(locked))]),
        [Told::Session(Some(Blocked::Locked))]
    );
    let away = SessionState {
        locked: Some(false),
        active: false,
    };
    assert_eq!(
        told(vec![WindowEvent::SessionRead(Ok(away))]),
        [Told::Session(Some(Blocked::Inactive))]
    );
}

#[test]
fn a_lock_and_an_unlock_each_tell_the_controller_however_fast() {
    assert_eq!(
        told(vec![
            WindowEvent::SessionRead(Ok(UNLOCKED)),
            lock(),
            unlock(),
            lock(),
            // Said twice: told once.
            lock(),
        ]),
        [
            Told::Session(None),
            Told::Session(Some(Blocked::Locked)),
            Told::Session(None),
            Told::Session(Some(Blocked::Locked)),
            Told::Nothing,
        ]
    );
}

#[test]
fn a_switch_to_another_user_and_back_in_either_order() {
    let away = vec![WindowEvent::SessionRead(Ok(UNLOCKED)), lock(), disconnect()];
    let mut back = away.clone_events();
    back.extend([connect(), unlock()]);
    assert_eq!(
        told(back),
        [
            Told::Session(None),
            Told::Session(Some(Blocked::Locked)),
            // Locked comes before another session in front.
            Told::Nothing,
            Told::Nothing,
            Told::Session(None),
        ]
    );
    let mut back = away.clone_events();
    back.extend([unlock(), connect()]);
    assert_eq!(
        told(back)[3..],
        // Only the session at the screen can be unlocked.
        [Told::Session(None), Told::Nothing]
    );
    // A remote desktop taking the session: disconnected, then connected.
    assert_eq!(
        told(vec![
            WindowEvent::SessionRead(Ok(UNLOCKED)),
            disconnect(),
            connect(),
        ]),
        [
            Told::Session(None),
            Told::Session(Some(Blocked::Inactive)),
            Told::Session(None),
        ]
    );
}

#[test]
fn a_state_not_known_refuses_until_a_notice_says_it() {
    let flags_unknown = SessionState {
        locked: None,
        active: true,
    };
    assert_eq!(
        told(vec![
            WindowEvent::SessionRead(Ok(flags_unknown)),
            lock(),
            unlock(),
        ]),
        [
            unknown("Windows does not say whether the session is locked"),
            Told::Session(Some(Blocked::Locked)),
            Told::Session(None),
        ]
    );
    let failed = || WindowEvent::SessionUnwatched(io::Error::other("not yet"));
    assert_eq!(
        told(vec![
            failed(),
            failed(),
            WindowEvent::SessionRead(Ok(UNLOCKED))
        ]),
        [
            unknown("Windows' session notices are not available: not yet"),
            Told::Nothing,
            Told::Session(None),
        ]
    );
    // A failed read forgets what was known.
    assert_eq!(
        told(vec![
            WindowEvent::SessionRead(Ok(UNLOCKED)),
            WindowEvent::SessionRead(Err(io::Error::other("no answer"))),
            // The lock alone does not say whether the session is in front.
            lock(),
        ]),
        [
            Told::Session(None),
            unknown("the session state could not be read: no answer"),
            Told::Nothing,
        ]
    );
    // Anything before the first read says so.
    assert_eq!(
        told(vec![WindowEvent::Display(true)]),
        [unknown("the session state is not read yet")]
    );
}

#[test]
fn sleep_the_display_turning_off_and_every_suspend_announce_sleep() {
    assert_eq!(
        told(vec![
            WindowEvent::SessionRead(Ok(UNLOCKED)),
            WindowEvent::Suspend,
            WindowEvent::Resume,
            WindowEvent::Display(false),
            // In the dark already, a suspend is announced again: the
            // thread waits for the controller before Windows suspends.
            WindowEvent::Suspend,
            lock(),
            WindowEvent::Resume,
            // Still dark.
            WindowEvent::Display(true),
            unlock(),
        ]),
        [
            Told::Session(None),
            Told::Sleep(1),
            Told::Session(None),
            Told::Sleep(2),
            Told::Sleep(3),
            Told::Nothing,
            Told::Nothing,
            Told::Session(Some(Blocked::Locked)),
            Told::Session(None),
        ]
    );
}

#[test]
fn a_display_that_cannot_be_followed_refuses_starts() {
    let unwatched = || WindowEvent::DisplayUnwatched(io::Error::other("refused"));
    let why = "the display's state cannot be followed: refused";
    assert_eq!(
        told(vec![
            WindowEvent::SessionRead(Ok(UNLOCKED)),
            unwatched(),
            // A state not known comes before the lock, as the session's
            // own does: starts stay refused for the same reason.
            lock(),
            unlock(),
        ]),
        [
            Told::Session(None),
            unknown(why),
            Told::Nothing,
            Told::Nothing,
        ]
    );
    // At start the display is registered for first: its failure arrives
    // before the session's first read, which then allows no start.
    assert_eq!(
        told(vec![unwatched(), WindowEvent::SessionRead(Ok(UNLOCKED))]),
        [unknown("the session state is not read yet"), unknown(why)]
    );
}

#[test]
fn other_events_tell_the_session_nothing() {
    assert_eq!(
        told(vec![
            WindowEvent::Other(0x1a),
            WindowEvent::Close,
            WindowEvent::EndSession,
            WindowEvent::Ended { failure: None },
        ]),
        [Told::Nothing, Told::Nothing, Told::Nothing, Told::Nothing]
    );
}

/// Copies of the session events a test starts from.
trait CloneEvents {
    fn clone_events(&self) -> Vec<WindowEvent>;
}

impl CloneEvents for Vec<WindowEvent> {
    fn clone_events(&self) -> Vec<WindowEvent> {
        self.iter()
            .map(|event| match event {
                WindowEvent::SessionRead(Ok(state)) => WindowEvent::SessionRead(Ok(*state)),
                WindowEvent::Session(change) => WindowEvent::Session(*change),
                other => panic!("not copied: {other:?}"),
            })
            .collect()
    }
}

#[test]
fn a_mark_wakes_its_waiter_when_raised() {
    let mark = Arc::new(Mark::default());
    assert!(mark.wait_for(0, Duration::ZERO));
    assert!(!mark.wait_for(1, Duration::from_millis(10)));
    let raiser = Arc::clone(&mark);
    let raising = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        raiser.raise(2);
    });
    let waiting = Instant::now();
    assert!(mark.wait_for(2, Duration::from_secs(30)));
    assert!(waiting.elapsed() < Duration::from_secs(10));
    raising.join().unwrap();
    // Never lowered.
    mark.raise(1);
    assert!(mark.wait_for(2, Duration::ZERO));
}

/// A handler on its own channel, and the mark it waits on.
fn notices() -> (Notices, mpsc::UnboundedReceiver<Message>, Arc<Mark>) {
    let (send, inbox) = mpsc::unbounded_channel();
    let released = Arc::new(Mark::default());
    let notices = Notices::new(send, Arc::clone(&released));
    (notices, inbox, released)
}

fn drained(inbox: &mut mpsc::UnboundedReceiver<Message>) -> Vec<Message> {
    std::iter::from_fn(|| inbox.try_recv().ok()).collect()
}

#[test]
fn a_close_logoff_or_the_threads_end_quits() {
    let (mut notices, mut inbox, released) = notices();
    notices.handle(WindowEvent::Close);
    notices.handle(WindowEvent::Ended {
        failure: Some("the message thread panicked: x".to_owned()),
    });
    released.raise(ENDED);
    let ending = Instant::now();
    notices.handle(WindowEvent::EndSession);
    assert!(ending.elapsed() < RELEASE_BEFORE_LOGOFF);
    notices.handle(WindowEvent::Other(0x1a));
    assert_eq!(drained(&mut inbox), vec![Message::Quit; 3]);
}

#[test]
fn sleep_waits_for_the_controller_to_let_go_and_no_longer() {
    let (mut notices, mut inbox, released) = notices();
    notices.handle(WindowEvent::SessionRead(Ok(UNLOCKED)));
    let raiser = Arc::clone(&released);
    let raising = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        raiser.raise(1);
    });
    let sleeping = Instant::now();
    notices.handle(WindowEvent::Suspend);
    assert!(sleeping.elapsed() < RELEASE_BEFORE_SLEEP);
    raising.join().unwrap();
    notices.handle(WindowEvent::Resume);
    // Nobody reports this one: the wait ends at its deadline.
    let sleeping = Instant::now();
    notices.handle(WindowEvent::Display(false));
    let waited = sleeping.elapsed();
    assert!(
        waited >= RELEASE_BEFORE_SLEEP && waited < RELEASE_BEFORE_SLEEP * 5,
        "{waited:?}"
    );
    // A suspend in the dark waits again, for a notice of its own.
    let suspending = Instant::now();
    notices.handle(WindowEvent::Suspend);
    let waited = suspending.elapsed();
    assert!(
        waited >= RELEASE_BEFORE_SLEEP && waited < RELEASE_BEFORE_SLEEP * 5,
        "{waited:?}"
    );
    // Once the controller has ended, nothing is held: no wait.
    released.raise(ENDED);
    notices.handle(WindowEvent::Resume);
    notices.handle(WindowEvent::Display(true));
    let suspending = Instant::now();
    notices.handle(WindowEvent::Suspend);
    assert!(suspending.elapsed() < RELEASE_BEFORE_SLEEP);
    assert_eq!(
        drained(&mut inbox),
        [
            Message::Session(None),
            Message::Sleep(1),
            Message::Session(None),
            Message::Sleep(2),
            Message::Sleep(3),
            Message::Session(None),
            Message::Sleep(4),
        ]
    );
}

/// A platform whose output is a fake that records what each run sent.
#[derive(Clone, Default)]
struct FakeOutput(Arc<Mutex<Vec<FakeLog>>>);

impl FakeOutput {
    /// What the `n`-th run sent.
    fn presses(&self, n: usize) -> Vec<PressState> {
        let logs = self.0.lock().unwrap();
        logs.get(n).map_or_else(Vec::new, |log| {
            log.calls()
                .into_iter()
                .map(|call| match call {
                    Call::Button(_, state) => state,
                    other => panic!("not a button: {other:?}"),
                })
                .collect()
        })
    }
}

impl Platform for FakeOutput {
    type Backend = FakeBackend;

    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    fn open_input(&mut self) -> OpenInput<FakeBackend> {
        let (output, log) = FakeBackend::new();
        self.0.lock().unwrap().push(log);
        Box::pin(async move { Ok(Input::new(output)) })
    }

    fn read_input(&mut self, _: Option<&Input<FakeBackend>>, _: ReadInput) -> HotkeyReader {
        Box::pin(pending())
    }

    fn event_clock(&self) -> Duration {
        Duration::ZERO
    }

    fn local_time(&self) -> Option<LocalTime> {
        None
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        None
    }
}

/// Waits, on the real clock, until `done` holds; panics after `within`.
async fn until(within: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A controller over `output` that holds the left button when started, on
/// a real clock, and how to reach it.
fn holding(output: FakeOutput) -> (Controller<FakeOutput>, Links) {
    let defaults = Config::default();
    let config = Config {
        tray_start_delay: Duration::ZERO,
        clicker: ClickerSettings {
            mode: Mode::HoldButton,
            ..defaults.clicker
        },
        ..defaults
    };
    Controller::new(output, None, config, None)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

/// How long a stop may take: the hold it ends would last for ever.
const AT_ONCE: Duration = Duration::from_secs(1);

#[test]
fn a_lock_from_the_message_thread_releases_a_held_button_at_once() {
    let output = FakeOutput::default();
    let (controller, links) = holding(output.clone());
    let mut handler = Notices::new(links.send.clone(), Arc::new(Mark::default()));
    handler.handle(WindowEvent::SessionRead(Ok(UNLOCKED)));
    // The message thread, as the daemon runs it: events on its own thread.
    let (events, heard) = std_mpsc::channel::<WindowEvent>();
    let message_thread = thread::spawn(move || {
        for event in heard {
            handler.handle(event);
        }
    });
    let view = links.view.clone();
    let send = links.send.clone();
    let steps = async {
        send.send(Message::Start).unwrap();
        until(Duration::from_secs(10), "holding", || {
            view.borrow().activity == Activity::Clicking && !output.presses(0).is_empty()
        })
        .await;
        let locking = Instant::now();
        events.send(lock()).unwrap();
        until(Duration::from_secs(10), "stopped", || {
            view.borrow().activity == Activity::Idle
        })
        .await;
        assert!(locking.elapsed() < AT_ONCE, "{:?}", locking.elapsed());
        assert_eq!(output.presses(0), [PressState::Down, PressState::Up]);
        send.send(Message::Quit).unwrap();
    };
    runtime().block_on(async { tokio::join!(controller.run(), steps) });
    drop(events);
    message_thread.join().unwrap();
}

#[test]
fn the_watchdogs_end_stops_a_hold_at_once_releases_and_quits() {
    let output = FakeOutput::default();
    let (controller, links) = holding(output.clone());
    let mut handler = Notices::new(links.send.clone(), Arc::new(Mark::default()));
    let ended = handler.watchdog_ended();
    handler.handle(WindowEvent::SessionRead(Ok(UNLOCKED)));
    let (events, heard) = std_mpsc::channel::<WindowEvent>();
    let message_thread = thread::spawn(move || {
        for event in heard {
            handler.handle(event);
        }
    });
    let view = links.view.clone();
    let send = links.send.clone();
    let steps = async {
        send.send(Message::Start).unwrap();
        until(Duration::from_secs(10), "holding", || {
            view.borrow().activity == Activity::Clicking && !output.presses(0).is_empty()
        })
        .await;
        let ending = Instant::now();
        events.send(WindowEvent::WatchdogEnded).unwrap();
        until(Duration::from_secs(10), "released", || {
            output.presses(0) == [PressState::Down, PressState::Up]
        })
        .await;
        assert!(ending.elapsed() < AT_ONCE, "{:?}", ending.elapsed());
    };
    // The controller ends on its own: the watchdog's end quits it.
    runtime().block_on(async { tokio::join!(controller.run(), steps) });
    assert!(ended.load(std::sync::atomic::Ordering::Acquire));
    drop(events);
    message_thread.join().unwrap();
}

/// Starts a hold, and once it holds, whether it took.
async fn try_holding(send: &mpsc::UnboundedSender<Message>, view: &watch::Receiver<View>) -> bool {
    send.send(Message::Start).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if view.borrow().activity == Activity::Clicking {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[test]
#[ignore = "locks the desktop: run in the WN01 VM through `vm run`, while the dev box types the \
            password 20 s in (`drive.py unlock` from X03's tools; see the PR)"]
fn locking_the_desktop_releases_a_held_button_at_once_and_unlocking_allows_starts() {
    let output = FakeOutput::default();
    let (controller, links) = holding(output.clone());
    let released = Arc::new(Mark::default());
    let mut handler = Notices::new(links.send.clone(), Arc::clone(&released));
    let thread = MessageThread::spawn(move |event| handler.handle(event), AT_ONCE * 5).unwrap();
    let view = links.view.clone();
    let send = links.send.clone();
    let steps = async {
        // Starts are refused until the message thread's first read.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !try_holding(&send, &view).await {
            assert!(Instant::now() < deadline, "never held");
        }
        until(Duration::from_secs(10), "holding", || {
            !output.presses(0).is_empty()
        })
        .await;
        let locking = Instant::now();
        let locked = std::process::Command::new("rundll32.exe")
            .arg("user32.dll,LockWorkStation")
            .status()
            .unwrap();
        assert!(locked.success());
        until(Duration::from_secs(10), "stopped", || {
            view.borrow().activity == Activity::Idle
        })
        .await;
        // Bounded from the call that locks, which returns before the lock
        // screen is up.
        let stopped = locking.elapsed();
        println!("stopped {stopped:?} after the lock was asked for");
        assert!(stopped < AT_ONCE * 2, "{stopped:?}");
        assert_eq!(output.presses(0), [PressState::Down, PressState::Up]);
        // While locked, a start is refused.
        assert!(!try_holding(&send, &view).await);
        // Once the password is typed, starts are allowed again.
        let deadline = Instant::now() + Duration::from_secs(120);
        while !try_holding(&send, &view).await {
            assert!(Instant::now() < deadline, "never unlocked");
        }
        println!("held again {:?} after the lock", locking.elapsed());
        // Quit only once the new hold has pressed, so it has a press to
        // release.
        until(Duration::from_secs(10), "held again", || {
            let runs = output.0.lock().unwrap().len();
            runs > 0 && !output.presses(runs - 1).is_empty()
        })
        .await;
        send.send(Message::Quit).unwrap();
    };
    runtime().block_on(async { tokio::join!(controller.run(), steps) });
    released.raise(ENDED);
    thread.quit_and_join(AT_ONCE).unwrap();
    let runs = output.0.lock().unwrap().len();
    assert_eq!(output.presses(runs - 1), [PressState::Down, PressState::Up]);
}

/// The command of the item labelled `label` in `menu`, or in its submenus.
fn command_of(menu: &[auto_clicker_backend_windows::MenuEntry], label: &str) -> Option<u32> {
    use auto_clicker_backend_windows::MenuEntry;
    menu.iter().find_map(|entry| match entry {
        MenuEntry::Item {
            label: shown,
            command,
            ..
        } if shown == label => *command,
        MenuEntry::Submenu { children, .. } => command_of(children, label),
        _ => None,
    })
}

#[test]
fn the_trays_choices_reach_the_controller_and_its_news_only_the_log() {
    use auto_clicker_core::locale::Language;

    use crate::tray_menu::tests::{file, view_listing};

    let (mut notices, mut inbox, _) = notices();
    let drawn = super::super::tray::content(
        &view_listing(Activity::Idle, 1),
        Language::English,
        16,
        &notices.commands(),
    );
    let start = command_of(&drawn.menu, "Start clicking").unwrap();
    let play = command_of(&drawn.menu, "Recording 2026-11-09 18:00:00").unwrap();
    notices.handle(WindowEvent::TrayMenuOpening);
    notices.handle(WindowEvent::TrayChosen(start));
    notices.handle(WindowEvent::TrayChosen(play));
    // A command no menu gave sends nothing.
    notices.handle(WindowEvent::TrayChosen(9999));
    notices.handle(WindowEvent::TrayMenuFailed(io::Error::other("no menu")));
    notices.handle(WindowEvent::TrayUnshown(io::Error::other("refused")));
    notices.handle(WindowEvent::TrayShown);
    assert_eq!(
        drained(&mut inbox),
        [Message::MenuOpened, Message::Start, Message::Play(file(0))]
    );
}

#[test]
fn a_refused_icon_is_logged_once_until_it_shows_again() {
    let (mut notices, mut inbox, _) = notices();
    let refused = || WindowEvent::TrayUnshown(io::Error::other("refused"));
    assert!(!notices.icon_refused);
    notices.handle(refused());
    assert!(notices.icon_refused);
    notices.handle(refused());
    notices.handle(WindowEvent::TrayShown);
    assert!(!notices.icon_refused);
    notices.handle(refused());
    assert!(notices.icon_refused);
    assert_eq!(drained(&mut inbox), []);
}
