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
//! waiting costs nothing.
//!
//! An activation says which shortcut fired and nothing else: no device and
//! no time. So this reader cannot leave out the program's own key output;
//! the controller keeps the program from pressing a hotkey's key instead,
//! from the triggers reported here.

mod portal;
mod reactor;

use std::future::Future;
use std::pin::pin;

use auto_clicker_core::hotkeys::{Bindings, HotkeyEvent, HotkeyStatus, Unavailable};
use futures_util::{Stream, StreamExt};

use portal::PortalShortcuts;

/// Reads hotkeys from the desktop's GlobalShortcuts portal and hands them
/// to `send`, with the desktop's triggers and whether hotkeys work. The
/// config's `bindings` are only what the program asks for at the first bind.
///
/// Runs until dropped, or until the session bus cannot be watched.
pub async fn read_shortcuts(bindings: Bindings, mut send: impl FnMut(HotkeyEvent)) {
    let owners = match portal::watch_owner().await {
        Ok(owners) => owners,
        Err(why) => return send(unavailable(why)),
    };
    run(&mut PortalShortcuts::new(bindings), owners, send).await;
}

/// Where shortcut sessions come from: the portal, or a test's script.
pub(crate) trait Desktop {
    /// Opens a session, binds the shortcuts and serves it, sending what it
    /// hears to `send`. Returns why the session ended; it does not end by
    /// itself otherwise. Dropping the future closes the session.
    fn session(&mut self, send: &mut dyn FnMut(HotkeyEvent)) -> impl Future<Output = String>;
}

/// What ended a session.
enum End {
    /// The session itself, with the reason.
    Session(String),
    /// A portal service's bus name changed hands: to an owner, or to none.
    /// `None` when the bus can no longer be watched.
    Owner(Option<bool>),
}

/// Serves one session after another. `owners` yields whether a portal
/// service has an owner, each time that changes for one of them.
///
/// A session a portal service took down is opened again at once, whether
/// the service is back or gone: the portal's services start when called and
/// not by themselves (KDE's is not restarted after a crash), so waiting for
/// one to return could wait for ever. A session that ended otherwise (a
/// declined dialog, a failed call) is opened again only after the next such
/// change, so nothing is retried in a loop.
pub(crate) async fn run(
    desktop: &mut impl Desktop,
    owners: impl Stream<Item = bool>,
    mut send: impl FnMut(HotkeyEvent),
) {
    let mut owners = pin!(owners);
    loop {
        // An owner change comes first: the session it ends is already dead.
        let end = tokio::select! {
            biased;
            change = owners.next() => End::Owner(change),
            why = desktop.session(&mut send) => End::Session(why),
        };
        let why = match end {
            End::Session(why) => {
                send(unavailable(why));
                End::Owner(owners.next().await)
            }
            changed @ End::Owner(_) => changed,
        };
        match why {
            End::Owner(Some(true)) => {
                send(unavailable(
                    "a portal service of the desktop restarted".to_owned(),
                ));
            }
            End::Owner(Some(false)) => {
                send(unavailable(
                    "a portal service of the desktop stopped".to_owned(),
                ));
            }
            End::Owner(None) | End::Session(_) => {
                return send(unavailable(
                    "the session bus can no longer be watched; restart auto-clicker".to_owned(),
                ));
            }
        }
    }
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
