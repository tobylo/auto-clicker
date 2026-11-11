//! Hotkeys through the desktop's GlobalShortcuts portal, without reading a
//! keyboard.
//!
//! The program registers its three shortcuts with the portal and the desktop
//! says when one is pressed. The desktop asks the user once, at the first
//! bind; from then on its own settings choose the keys, and the program
//! only hears which ones they are. A new session gets no activations until
//! it binds, so [`read_shortcuts`] binds every time it starts.
//!
//! One session serves until the portal goes away. Every way of losing it
//! (a portal service leaving the bus, the session closed, a call that
//! fails) is reported as hotkeys being unavailable. The reader binds anew
//! when a portal service's bus name changes hands, never on a timer, so
//! waiting costs nothing; after five sessions in a row that never bound, it
//! gives up until the daemon restarts.
//!
//! An activation says which shortcut fired and nothing else: no device and
//! no time. So this reader cannot leave out the program's own key output;
//! the controller keeps the program from pressing a hotkey's key instead,
//! from the triggers reported here.

mod portal;
mod reactor;

use std::cell::Cell;
use std::future::Future;
use std::pin::pin;

use auto_clicker_core::controller::{ReaderCommand, ReaderCommands};
use auto_clicker_core::hotkeys::{Bindings, HotkeyEvent, HotkeyStatus, Unavailable};
use futures_util::{Stream, StreamExt};

use portal::PortalShortcuts;

/// Reads hotkeys from the desktop's GlobalShortcuts portal and hands them
/// to `send`, with the desktop's triggers and whether hotkeys work. The
/// config's `bindings` are only what the program asks for at the first bind.
/// `commands` ask it to open the desktop's settings for the shortcuts; it
/// answers each with [`HotkeyEvent::Configured`].
///
/// Runs until dropped, or until the session bus cannot be watched.
pub async fn read_shortcuts(
    bindings: Bindings,
    mut commands: ReaderCommands,
    mut send: impl FnMut(HotkeyEvent),
) {
    let owners = match portal::watch_owner().await {
        Ok(owners) => owners,
        Err(why) => return send(unavailable(why)),
    };
    run(
        &mut PortalShortcuts::new(bindings),
        owners,
        &mut commands,
        send,
    )
    .await;
}

/// Where shortcut sessions come from: the portal, or a test's script.
pub(crate) trait Desktop {
    /// Opens a session, binds the shortcuts and serves it, sending what it
    /// hears to `send` and answering `commands` once bound. Returns why the
    /// session ended; it does not end by itself otherwise. Dropping the
    /// future closes the session.
    fn session(
        &mut self,
        send: &mut dyn FnMut(HotkeyEvent),
        commands: &mut ReaderCommands,
    ) -> impl Future<Output = String>;
}

/// The answer to a command while no session is bound.
pub(crate) const NOT_BOUND: &str =
    "the desktop's shortcuts are not bound now; see the tray for why";

/// What ended a session.
enum End {
    /// The session itself, with the reason.
    Session(String),
    /// A portal service's bus name changed hands: to an owner, or to none.
    /// `None` when the bus can no longer be watched.
    Owner(Option<bool>),
}

/// How many sessions in a row may end without having bound the shortcuts
/// before the reader gives up. Opening a session starts a portal service
/// that is not running, so a service that crashes on every bind would
/// otherwise be started again for ever.
pub(crate) const MAX_FAILED_IN_A_ROW: u32 = 5;

/// Serves one session after another. `owners` yields whether a portal
/// service has an owner, each time that changes for one of them.
///
/// A session a portal service took down is opened again at once, whether
/// the service is back or gone: the portal's services start when called and
/// not by themselves (KDE's is not restarted after a crash), so waiting for
/// one to return could wait for ever. A session that ended otherwise (a
/// declined dialog, a failed call) is opened again only after the next such
/// change. Nothing is tried on a timer, and after
/// [`MAX_FAILED_IN_A_ROW`] sessions that never bound, nothing is tried at
/// all.
pub(crate) async fn run(
    desktop: &mut impl Desktop,
    owners: impl Stream<Item = bool>,
    commands: &mut ReaderCommands,
    mut send: impl FnMut(HotkeyEvent),
) {
    let mut owners = pin!(owners);
    let mut failed_in_a_row = 0u32;
    loop {
        let (end, bound) = one_session(desktop, &mut owners, commands, &mut send).await;
        failed_in_a_row = failures_after(failed_in_a_row, bound);
        let gave_up = failed_in_a_row >= MAX_FAILED_IN_A_ROW;
        let waiting = Waiting {
            owners: &mut owners,
            commands: &mut *commands,
        };
        let why = reason(end, gave_up, waiting, &mut send).await;
        let (report, done) = verdict(why, gave_up);
        send(unavailable(report));
        if done {
            return;
        }
    }
}

/// How many sessions in a row have failed to bind, after one more.
fn failures_after(before: u32, bound: bool) -> u32 {
    if bound { 0 } else { before.saturating_add(1) }
}

/// What a reader between sessions waits on.
struct Waiting<'a, O> {
    owners: &'a mut O,
    commands: &'a mut ReaderCommands,
}

impl<O: Stream<Item = bool> + Unpin> Waiting<'_, O> {
    /// The next change of a portal service's owner, with no session bound.
    async fn owner_change(&mut self, send: &mut impl FnMut(HotkeyEvent)) -> Option<bool> {
        unbound(self.owners.next(), self.commands, send).await
    }
}

/// Runs `work` while no session is bound (between sessions, or while one
/// binds, which can wait minutes on the desktop's dialog), answering every
/// command meanwhile at once: there are no settings to open yet, and a
/// command kept for later would open them long after it was answered.
pub(crate) async fn unbound<T>(
    work: impl Future<Output = T>,
    commands: &mut ReaderCommands,
    send: &mut dyn FnMut(HotkeyEvent),
) -> T {
    let mut work = pin!(work);
    loop {
        tokio::select! {
            biased;
            done = &mut work => return done,
            command = commands.next() => {
                let ReaderCommand::ConfigureShortcuts { operation, .. } = command;
                let result = Err(NOT_BOUND.to_owned());
                send(HotkeyEvent::Configured { operation, result });
            }
        }
    }
}

/// Why the next session is opened, once it may be: a session that ended by
/// itself waits here for a portal service to change first, unless the
/// reader has given up. `None` when the bus can no longer be watched.
async fn reason(
    end: End,
    gave_up: bool,
    mut waiting: Waiting<'_, impl Stream<Item = bool> + Unpin>,
    send: &mut impl FnMut(HotkeyEvent),
) -> Option<String> {
    match end {
        End::Session(why) if gave_up => Some(why),
        End::Session(why) => {
            send(unavailable(why));
            owner_change(waiting.owner_change(send).await)
        }
        End::Owner(change) => owner_change(change),
    }
}

/// What to report before the next session, and whether there is none.
fn verdict(why: Option<String>, gave_up: bool) -> (String, bool) {
    match why {
        None => (
            "the session bus can no longer be watched; restart auto-clicker".to_owned(),
            true,
        ),
        Some(why) if gave_up => (
            format!(
                "the desktop's shortcuts could not be bound in {MAX_FAILED_IN_A_ROW} tries (the \
                 last: {why}); restart auto-clicker to try again"
            ),
            true,
        ),
        Some(why) => (why, false),
    }
}

/// Runs one session until it ends or a portal service changes hands;
/// whether it got as far as binding the shortcuts.
async fn one_session(
    desktop: &mut impl Desktop,
    owners: &mut (impl Stream<Item = bool> + Unpin),
    commands: &mut ReaderCommands,
    send: &mut impl FnMut(HotkeyEvent),
) -> (End, bool) {
    let bound = Cell::new(false);
    let mut pass = |event: HotkeyEvent| {
        // A session reports that hotkeys work once it has bound them.
        if event == HotkeyEvent::Status(HotkeyStatus::Available) {
            bound.set(true);
        }
        send(event);
    };
    // An owner change comes first: the session it ends is already dead.
    let end = tokio::select! {
        biased;
        change = owners.next() => End::Owner(change),
        why = desktop.session(&mut pass, commands) => End::Session(why),
    };
    (end, bound.get())
}

/// What a change of a portal service's owner means for the session it
/// ended; `None` when the bus can no longer be watched.
fn owner_change(change: Option<bool>) -> Option<String> {
    let what = if change? { "restarted" } else { "stopped" };
    Some(format!("a portal service of the desktop {what}"))
}

/// The longest reason shown, in characters.
const MAX_REASON: usize = 240;

/// Hotkeys are unavailable for this reason. It may quote the portal's own
/// error text, so it is shown without control characters and bounded.
fn unavailable(why: String) -> HotkeyEvent {
    let why = why
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_REASON)
        .collect();
    HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(why)))
}

#[cfg(test)]
mod tests;
