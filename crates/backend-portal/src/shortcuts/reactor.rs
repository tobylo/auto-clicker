//! One bound session: turning the portal's signals into hotkey events.

use std::future::{Future, pending, poll_fn};
use std::pin::Pin;
use std::pin::pin;
use std::task::Poll;

use auto_clicker_core::controller::{ReaderCommand, ReaderCommands};
use auto_clicker_core::hotkeys::{
    Action, DESKTOP_RELEASE_PAUSE, HotkeyEvent, HotkeyStatus, Trigger, Triggers,
};
use futures_util::{Stream, StreamExt};
use tokio::time::{Instant, sleep_until};

/// The id each action is registered under. The desktop stores them with the
/// user's triggers, so they never change.
const IDS: [(Action, &str); 3] = [
    (Action::ToggleClicking, "toggle_clicking"),
    (Action::ToggleRecording, "toggle_recording"),
    (Action::TogglePlayback, "toggle_playback"),
];

pub(super) fn id_of(action: Action) -> &'static str {
    match action {
        Action::ToggleClicking => IDS[0].1,
        Action::ToggleRecording => IDS[1].1,
        Action::TogglePlayback => IDS[2].1,
    }
}

fn action_of(id: &str) -> Option<Action> {
    IDS.into_iter()
        .find(|(_, known)| *known == id)
        .map(|(action, _)| action)
}

/// A shortcut as the portal lists it: its id and its trigger's description.
pub(super) type Listed = (String, String);

/// What a session heard from the portal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Signal {
    /// The shortcut with this id was pressed.
    Activated(String),
    /// The shortcut with this id was let go: the first of its keys came up.
    Deactivated(String),
    /// These shortcuts have new triggers.
    Changed(Vec<Listed>),
    /// The portal closed the session.
    Closed,
    /// A signal of the shortcuts that did not parse; its name. What it
    /// said is lost, so the session can no longer be trusted.
    Malformed(String),
}

/// The triggers after a bind that answered with `listed`. A shortcut the
/// answer leaves out is not bound: it has no trigger.
pub(super) fn bound_triggers(listed: &[Listed]) -> Triggers {
    let mut triggers = Triggers::unknown();
    for (action, _) in IDS {
        triggers.set(action, Trigger::reported(""));
    }
    apply(&mut triggers, listed);
    triggers
}

fn apply(triggers: &mut Triggers, listed: &[Listed]) {
    for (id, description) in listed {
        if let Some(action) = action_of(id) {
            triggers.set(action, Trigger::reported(description));
        }
    }
}

/// The state of one bound session.
struct Session {
    triggers: Triggers,
    /// A hotkey fired and its release was not reported yet.
    awaiting_release: bool,
    /// When to report the release: a pause after the desktop's own, which
    /// comes while the modifiers may still be down.
    release_at: Option<Instant>,
}

impl Session {
    /// A press. The desktop sends one activation per press, however long
    /// the keys are held, and releases one shortcut before it activates
    /// another (measured on Plasma 6.8.2), so every activation fires.
    fn activated(&mut self, id: &str) -> Option<HotkeyEvent> {
        let action = action_of(id)?;
        self.awaiting_release = true;
        // Fires before the release share one, counted from the last.
        self.release_at = None;
        Some(HotkeyEvent::Fired(action))
    }

    fn deactivated(&mut self, now: Instant) {
        if self.awaiting_release {
            self.release_at = now.checked_add(DESKTOP_RELEASE_PAUSE);
        }
    }

    fn release_due(&mut self) -> HotkeyEvent {
        self.release_at = None;
        self.awaiting_release = false;
        HotkeyEvent::ModifiersReleased
    }

    fn changed(&mut self, listed: &[Listed]) -> HotkeyEvent {
        apply(&mut self.triggers, listed);
        HotkeyEvent::Triggers(self.triggers.clone())
    }
}

/// What woke a session.
enum Wake {
    Signal(Option<Signal>),
    ReleaseDue,
    Command(ReaderCommand),
    /// The call opening the desktop's settings for this operation ended.
    Configured(u64, Result<(), String>),
}

/// The call opening the desktop's settings, while it runs, and the
/// operation it answers.
type Configuring<'a> = Option<(u64, Pin<Box<dyn Future<Output = Result<(), String>> + 'a>>)>;

/// The end of the call in flight; never resolves while none is.
fn configure_end<'a>(
    configuring: &'a mut Configuring<'_>,
) -> impl Future<Output = (u64, Result<(), String>)> + 'a {
    poll_fn(move |cx| {
        let Some((operation, call)) = configuring else {
            return Poll::Pending;
        };
        let operation = *operation;
        call.as_mut().poll(cx).map(|result| {
            *configuring = None;
            (operation, result)
        })
    })
}

/// What answers a request for the desktop's settings while another is
/// still being opened; the controller asks one at a time, so only a
/// controller that gave up on the first one sees it.
pub(super) const STILL_OPENING: &str = "the desktop's settings are still being opened";

/// What a bound session serves with: its signals, the controller's
/// commands, and how it opens the desktop's settings for its shortcuts,
/// with an activation token if there is one.
pub(super) struct Served<'a, S, C> {
    pub(super) signals: S,
    pub(super) commands: &'a mut ReaderCommands,
    pub(super) configure: C,
}

/// Serves a session bound with `triggers` until it is lost: reports the
/// triggers and that hotkeys work, then every hotkey its signals bring, and
/// answers each command. The call a command makes runs beside the hotkeys,
/// so a stop pressed meanwhile is not held up; it ends with the session.
/// Returns why the session ended.
pub(super) async fn serve<C>(
    triggers: Triggers,
    served: Served<'_, impl Stream<Item = Signal>, C>,
    send: &mut dyn FnMut(HotkeyEvent),
) -> String
where
    C: AsyncFn(Option<String>) -> Result<(), String>,
{
    let Served {
        signals,
        commands,
        configure,
    } = served;
    let mut configuring: Configuring<'_> = None;
    send(HotkeyEvent::Triggers(triggers.clone()));
    send(HotkeyEvent::Status(HotkeyStatus::Available));
    let mut session = Session {
        triggers,
        awaiting_release: false,
        release_at: None,
    };
    let mut signals = pin!(signals);
    loop {
        // Signals first: a press that arrives with a due release cancels it.
        let wake = tokio::select! {
            biased;
            signal = signals.next() => Wake::Signal(signal),
            () = due(session.release_at) => Wake::ReleaseDue,
            (operation, result) = configure_end(&mut configuring) => {
                Wake::Configured(operation, result)
            }
            command = commands.next() => Wake::Command(command),
        };
        let event = match wake {
            Wake::ReleaseDue => Some(session.release_due()),
            Wake::Configured(operation, result) => {
                Some(HotkeyEvent::Configured { operation, result })
            }
            Wake::Command(ReaderCommand::ConfigureShortcuts {
                operation,
                activation_token,
            }) => match configuring {
                Some(_) => Some(HotkeyEvent::Configured {
                    operation,
                    result: Err(STILL_OPENING.to_owned()),
                }),
                None => {
                    configuring = Some((operation, Box::pin(configure(activation_token))));
                    None
                }
            },
            Wake::Signal(Some(Signal::Activated(id))) => session.activated(&id),
            Wake::Signal(Some(Signal::Deactivated(_))) => {
                session.deactivated(Instant::now());
                None
            }
            Wake::Signal(Some(Signal::Changed(listed))) => Some(session.changed(&listed)),
            Wake::Signal(Some(Signal::Closed)) => {
                return "the desktop closed the shortcuts session".to_owned();
            }
            Wake::Signal(Some(Signal::Malformed(name))) => {
                return format!("the desktop sent a {name} signal the program cannot read");
            }
            Wake::Signal(None) => return "the session bus connection ended".to_owned(),
        };
        if let Some(event) = event {
            send(event);
        }
    }
}

async fn due(at: Option<Instant>) {
    match at {
        Some(at) => sleep_until(at).await,
        None => pending().await,
    }
}
