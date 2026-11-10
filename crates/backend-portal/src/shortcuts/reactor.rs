//! One bound session: turning the portal's signals into hotkey events.

use std::future::pending;
use std::pin::pin;

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
    /// The shortcut that is down. The desktop may repeat its activation
    /// while it is held; only the first counts. Any release clears it, so a
    /// lost one cannot block the hotkey for longer than one press.
    held: Option<Action>,
    /// A hotkey fired and its release was not reported yet.
    awaiting_release: bool,
    /// When to report the release: a pause after the desktop's own, which
    /// comes while the modifiers may still be down.
    release_at: Option<Instant>,
}

impl Session {
    fn activated(&mut self, id: &str) -> Option<HotkeyEvent> {
        let action = action_of(id)?;
        if self.held == Some(action) {
            return None;
        }
        self.held = Some(action);
        self.awaiting_release = true;
        // Fires before the release share one, counted from the last.
        self.release_at = None;
        Some(HotkeyEvent::Fired(action))
    }

    fn deactivated(&mut self, now: Instant) {
        self.held = None;
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
        // A trigger that changed while its keys were down is not held.
        self.held = None;
        apply(&mut self.triggers, listed);
        HotkeyEvent::Triggers(self.triggers.clone())
    }
}

/// What woke a session.
enum Wake {
    Signal(Option<Signal>),
    ReleaseDue,
}

/// Serves a session bound with `triggers` until it is lost: reports the
/// triggers and that hotkeys work, then every hotkey `signals` brings.
/// Returns why the session ended.
pub(super) async fn serve(
    triggers: Triggers,
    signals: impl Stream<Item = Signal>,
    send: &mut dyn FnMut(HotkeyEvent),
) -> String {
    send(HotkeyEvent::Triggers(triggers.clone()));
    send(HotkeyEvent::Status(HotkeyStatus::Available));
    let mut session = Session {
        triggers,
        held: None,
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
        };
        let event = match wake {
            Wake::ReleaseDue => Some(session.release_due()),
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
