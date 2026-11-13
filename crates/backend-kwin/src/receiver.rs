//! The receiver: the interface the scripts call, on the daemon's own
//! connection and bus name (brief decisions 3, 5 and 9).
//!
//! zbus runs it with `spawn = false`, so calls are handled one at a time, in
//! the order they arrived, on the connection's own task. A handler does no
//! waiting: it stamps the call, checks that it came from the current owner
//! of `org.kde.KWin` with the token of a current load, parses it, and hands
//! it to the watcher through a bounded queue without blocking. When the
//! queue is full (the daemon's thread did not get to it) the report is
//! dropped: a recording then has a gap in its positions and carries on. A
//! dropped layout report also tells the watcher, which then judges no
//! position by an older layout and loads the position script again.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::sync::mpsc::{self, error::TrySendError};
use zbus::interface;
use zbus::message::Header;

use crate::report::{self, Body, Report};
use crate::token::Token;

/// The interface the scripts call, as `#[interface]` below names it.
#[cfg(test)]
pub(crate) const INTERFACE: &str = "io.github.tobylo.AutoClicker.KWin";
/// Where it is served.
pub(crate) const PATH: &str = "/io/github/tobylo/AutoClicker/KWin";
/// Reports the queue holds: about a second of a 1000 Hz mouse.
pub(crate) const QUEUE: usize = 1024;

/// Whom calls are accepted from: the watcher sets it, the handlers read it.
#[derive(Debug, Default)]
pub(crate) struct Trust {
    /// The unique name owning `org.kde.KWin`, as last seen.
    pub(crate) owner: Option<String>,
    /// The tokens of the loads now current, at most one per script.
    pub(crate) tokens: Vec<Token>,
}

/// Shared between the watcher and the handlers, which zbus requires to be
/// `Send`. Never held across a wait.
pub(crate) type SharedTrust = Arc<Mutex<Trust>>;

/// Runs `with` on the trust; a panic elsewhere while it was held leaves it
/// as it was, which is still a value either side may read.
pub(crate) fn with_trust<R>(trust: &SharedTrust, with: impl FnOnce(&mut Trust) -> R) -> R {
    with(&mut trust.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The handlers' side.
pub(crate) struct Receiver {
    pub(crate) trust: SharedTrust,
    /// The event clock.
    pub(crate) clock: fn() -> Duration,
    pub(crate) queue: mpsc::Sender<Report>,
    /// Fired when a layout report did not fit in the queue.
    pub(crate) layout_dropped: Arc<Notify>,
}

/// One call as it arrived: who sent it, the token it carries and its text.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Call<'a> {
    pub(crate) sender: Option<&'a str>,
    pub(crate) token: &'a str,
    pub(crate) text: &'a str,
}

impl Receiver {
    /// A position call.
    pub(crate) fn position_from(&self, call: Call<'_>) {
        self.accept(call, |text| report::point(text).map(Body::Position));
    }

    /// A layout call.
    pub(crate) fn layout_from(&self, call: Call<'_>) {
        self.accept(call, |text| Some(Body::Layout(report::layout(text))));
    }

    /// Accepts `call`, whose text `parse` reads. The clock is read first, so
    /// the stamp holds no checking or parsing; the sender and the token are
    /// checked before the text is read.
    fn accept(&self, call: Call<'_>, parse: fn(&str) -> Option<Body>) {
        let time = (self.clock)();
        let Some(token) = self.trusted(call) else {
            return;
        };
        let Some(body) = parse(call.text) else {
            return;
        };
        let report = Report { time, token, body };
        if let Err(TrySendError::Full(report)) = self.queue.try_send(report)
            && matches!(report.body, Body::Layout(_))
        {
            self.layout_dropped.notify_one();
        }
    }

    /// The call's token, when its sender owns `org.kde.KWin` and the token
    /// is a current load's.
    fn trusted(&self, call: Call<'_>) -> Option<Token> {
        let token = Token::parse(call.token)?;
        with_trust(&self.trust, |trust| {
            let owner = trust.owner.as_deref();
            let from_owner = owner.is_some() && call.sender == owner;
            (from_owner && trust.tokens.contains(&token)).then_some(token)
        })
    }
}

#[interface(name = "io.github.tobylo.AutoClicker.KWin", spawn = false)]
impl Receiver {
    /// The pointer, as `"x y"` in logical px.
    fn position(&self, #[zbus(header)] header: Header<'_>, token: &str, point: &str) {
        let sender = header.sender().map(|name| name.as_str());
        self.position_from(Call {
            sender,
            token,
            text: point,
        });
    }

    /// The screens, as `"x y width height scale"` per output, `;`-separated.
    fn layout(&self, #[zbus(header)] header: Header<'_>, token: &str, outputs: &str) {
        let sender = header.sender().map(|name| name.as_str());
        self.layout_from(Call {
            sender,
            token,
            text: outputs,
        });
    }
}

#[cfg(test)]
mod tests;
