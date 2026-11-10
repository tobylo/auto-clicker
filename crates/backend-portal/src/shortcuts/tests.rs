//! The shortcut reader against scripted sessions and signals, on tokio's
//! paused clock: no portal is needed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::pending;
use std::rc::Rc;
use std::time::Duration;

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
    session_namespace, within,
};
use super::reactor::{Listed, Signal, bound_triggers, id_of, serve};
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
        why.to_owned(),
    )))
}

/// Lets the reader handle what was sent, without moving the clock.
async fn settle() {
    for _ in 0..20 {
        yield_now().await;
    }
}

/// One served session: its signals go in, its events and its end come out.
struct Served {
    signals: mpsc::UnboundedSender<Signal>,
    events: Rc<RefCell<Vec<HotkeyEvent>>>,
    end: tokio::task::JoinHandle<String>,
}

impl Served {
    /// Serves a session bound with the default triggers; the two events
    /// every session opens with are checked and left out.
    async fn start() -> Served {
        let (signals, received) = mpsc::unbounded_channel();
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let end = tokio::task::spawn_local(async move {
            let signals = futures_util::stream::unfold(received, |mut received| async {
                received.recv().await.map(|signal| (signal, received))
            });
            let mut send = |event| sink.borrow_mut().push(event);
            serve(bound_triggers(&defaults()), signals, &mut send).await
        });
        settle().await;
        let served = Served {
            signals,
            events,
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
