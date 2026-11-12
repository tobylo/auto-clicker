//! The shortcut reader against scripted sessions and signals, on tokio's
//! paused clock: no portal is needed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::pending;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::controller::{ReaderCommand, ReaderCommands};
use auto_clicker_core::hotkeys::{
    Action, Binding, DESKTOP_RELEASE_PAUSE, HotkeyEvent, HotkeyStatus, Trigger, Triggers,
    Unavailable,
};
use auto_clicker_core::input::InputError;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::{Instant, advance};
use zbus::zvariant::{ObjectPath, Value};

use super::portal::{
    MAX_LISTED, closes_a_session, connect_failure, failed, has_owner, parse, preferred_trigger,
    sent_by, session_namespace, within,
};
use super::reactor::{
    Listed, STILL_OPENING, Served as Serving, Signal, bound_triggers, id_of, serve,
};
use super::*;

const CLICKING: &str = "toggle_clicking";
const RECORDING: &str = "toggle_recording";

fn listed(pairs: &[(&str, &str)]) -> Vec<Listed> {
    pairs
        .iter()
        .map(|(id, trigger)| ((*id).to_owned(), (*trigger).to_owned()))
        .collect()
}

fn defaults() -> Vec<Listed> {
    listed(&[
        (CLICKING, "Ctrl+Shift+F6"),
        (RECORDING, "Ctrl+Shift+F7"),
        ("toggle_playback", "Ctrl+Shift+F8"),
    ])
}

fn fired(action: Action) -> HotkeyEvent {
    HotkeyEvent::Fired(action)
}

fn lost(why: &str) -> HotkeyEvent {
    HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(
        why.to_owned().into(),
    )))
}

/// Lets the reader handle what was sent, without moving the clock.
async fn settle() {
    for _ in 0..20 {
        yield_now().await;
    }
}

/// One served session: its signals and commands go in, its events, the
/// activation tokens it opened the desktop's settings with, and its end
/// come out. Opening the settings answers what `configured` holds.
struct Served {
    signals: mpsc::UnboundedSender<Signal>,
    commands: mpsc::UnboundedSender<ReaderCommand>,
    events: Rc<RefCell<Vec<HotkeyEvent>>>,
    tokens: Rc<RefCell<Vec<Option<String>>>>,
    configured: Rc<RefCell<Result<(), String>>>,
    /// Opening the settings never ends, as on a portal that stalls.
    stalls: Rc<std::cell::Cell<bool>>,
    end: tokio::task::JoinHandle<String>,
}

impl Served {
    /// Serves a session bound with the default triggers; the two events
    /// every session opens with are checked and left out.
    async fn start() -> Served {
        let (signals, received) = mpsc::unbounded_channel();
        let (commands, mut asked) = ReaderCommands::channel();
        let events = Rc::new(RefCell::new(Vec::new()));
        let tokens = Rc::new(RefCell::new(Vec::new()));
        let configured = Rc::new(RefCell::new(Ok(())));
        let stalls = Rc::new(std::cell::Cell::new(false));
        let (sink, noted, answer) = (events.clone(), tokens.clone(), configured.clone());
        let stall = stalls.clone();
        let end = tokio::task::spawn_local(async move {
            let signals = futures_util::stream::unfold(received, |mut received| async {
                received.recv().await.map(|signal| (signal, received))
            });
            let mut send = |event| sink.borrow_mut().push(event);
            let serving = Serving {
                signals,
                commands: &mut asked,
                configure: async |token| {
                    noted.borrow_mut().push(token);
                    if stall.get() {
                        pending::<()>().await;
                    }
                    answer.borrow().clone()
                },
            };
            serve(bound_triggers(&defaults()), serving, &mut send).await
        });
        settle().await;
        let served = Served {
            signals,
            commands,
            events,
            tokens,
            configured,
            stalls,
            end,
        };
        assert_eq!(
            served.take(),
            [
                HotkeyEvent::Triggers(bound_triggers(&defaults())),
                HotkeyEvent::Status(HotkeyStatus::Available),
            ]
        );
        served
    }

    async fn hear(&self, signal: Signal) {
        self.signals.send(signal).unwrap();
        settle().await;
    }

    async fn press(&self, id: &str) {
        self.hear(Signal::Activated(id.to_owned())).await;
    }

    async fn let_go(&self, id: &str) {
        self.hear(Signal::Deactivated(id.to_owned())).await;
    }

    /// The events since the last call.
    fn take(&self) -> Vec<HotkeyEvent> {
        self.events.borrow_mut().drain(..).collect()
    }
}

async fn local(body: impl Future<Output = ()>) {
    tokio::task::LocalSet::new().run_until(body).await;
}

mod served;
mod sessions;
mod wire;
