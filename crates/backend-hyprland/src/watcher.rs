//! The watcher: polled on the daemon's thread beside the controller, it
//! asks Hyprland where the pointer is while a recording runs, and hands
//! each answer to the recorder with the layout it was asked on.
//!
//! - **When it asks (W7).** Once when a recording starts, then on motion
//!   (the slot's motion notice): at once when no ask is in flight and the
//!   last went out at least [`CAP`] ago, else once more as soon as both
//!   hold. So it asks at most once per millisecond, once after the last
//!   move it held back, and never while idle. No timer runs but that one
//!   deadline, set by motion.
//! - **One in flight (W6).** Each ask has its own deadline ([`ask`]); a
//!   stop or a new recording drops the ask in flight, its connection and
//!   everything held.
//! - **The layout (W4).** An answer is stamped when it is read. It is
//!   dropped when the layout changed since its ask went out (Hyprland
//!   holds answers during a modeset and then answers on the new layout),
//!   and otherwise held [`HOLD`] before it is fed, and dropped when the
//!   layout changes meanwhile: Hyprland's answer on new screens can come
//!   up to 3.12 ms before the display says the screens changed (decision
//!   0004). From the moment the display tells of a change until a layout
//!   settles it, the layout counts as changed: an ask sent then, and an
//!   answer held then, are dropped, however long the daemon was busy. It is fed with the layout it was asked on, moved by that
//!   layout's corner.
//! - **The reason.** An ask that fails sets why, and one answered clears
//!   it; a recording's stop clears what its own asks set (W01 brief,
//!   "Settled by W01d").
//!
//! [`ask`]: crate::ask

use std::collections::VecDeque;
use std::future::{Future, pending};
use std::pin::Pin;
use std::time::Duration;

use auto_clicker_backend_wayland::Snapshot;
use auto_clicker_core::recording::{Layout, Point, PointerSource, RecordingFeed, Taken};
use tokio::time::{Instant, sleep_until};

use crate::Reason;
use crate::ask::Failure;
use crate::reply::Global;

/// The shortest time between two asks.
pub(crate) const CAP: Duration = Duration::from_millis(1);

/// How long an answer waits before it is fed, for a layout change it may
/// belong to: three times the longest the display was measured late
/// (decision 0004).
pub(crate) const HOLD: Duration = Duration::from_millis(10);

/// The layout as the watcher reads it ([`Wayland`] in the daemon).
///
/// [`Wayland`]: auto_clicker_backend_wayland::Wayland
pub(crate) trait Screens {
    fn snapshot(&self) -> Snapshot;
    fn changes(&self) -> Option<u64>;
}

impl Screens for auto_clicker_backend_wayland::Wayland {
    fn snapshot(&self) -> Snapshot {
        auto_clicker_backend_wayland::Wayland::snapshot(self)
    }

    fn changes(&self) -> Option<u64> {
        auto_clicker_backend_wayland::Wayland::changes(self)
    }
}

/// One ask in flight.
pub(crate) type Asking = Pin<Box<dyn Future<Output = Result<Global, Failure>>>>;

/// How the watcher asks: over the socket in the daemon.
pub(crate) trait Asker {
    fn ask(&self) -> Asking;
}

/// An answer waiting out [`HOLD`].
struct Held {
    until: Instant,
    stamp: Duration,
    point: Point,
    layout: Layout,
    changes: u64,
}

/// What woke the watcher.
enum Wake {
    Recording,
    Answer(Result<Global, Failure>),
    Fed,
    Ask,
    Motion,
}

pub(crate) struct Watcher<S, A> {
    pub(crate) screens: S,
    pub(crate) asker: A,
    pub(crate) feed: RecordingFeed,
    pub(crate) reason: Reason,
    /// The event clock, which stamps each answer.
    pub(crate) clock: fn() -> Duration,
}

/// What one recording's asks have under way.
#[derive(Default)]
struct Round {
    /// The ask in flight, with the layout when it went out.
    asking: Option<(Asking, Snapshot)>,
    last_sent: Option<Instant>,
    /// Motion came that no ask has followed yet.
    held_back: bool,
    held: VecDeque<Held>,
    /// One of this recording's asks set the reason.
    set_reason: bool,
}

impl<S: Screens, A: Asker> Watcher<S, A> {
    pub(crate) async fn run(self) {
        let mut changes = self.feed.clone();
        let mut round = Round::default();
        loop {
            let feed_at = round.held.front().map(|held| held.until);
            let ask_at = round.next_ask();
            let wake = tokio::select! {
                biased;
                () = changes.changed() => Wake::Recording,
                answer = asked(&mut round.asking) => Wake::Answer(answer),
                () = until(feed_at) => Wake::Fed,
                () = until(ask_at) => Wake::Ask,
                () = self.feed.motion() => Wake::Motion,
            };
            match wake {
                Wake::Recording => round = self.recording_changed(round),
                Wake::Answer(answer) => self.answered(&mut round, answer),
                Wake::Fed => self.feed_held(&mut round),
                Wake::Ask => self.send(&mut round),
                Wake::Motion => self.moved(&mut round),
            }
        }
    }

    /// A recording started or stopped: what the last one had under way
    /// ends, and a new one is asked at once.
    fn recording_changed(&self, ended: Round) -> Round {
        if ended.set_reason {
            self.reason.set(Ok(()));
        }
        let mut round = Round::default();
        if self.feed.recording() {
            self.send(&mut round);
        }
        round
    }

    fn moved(&self, round: &mut Round) {
        if !self.feed.recording() {
            return;
        }
        let capped = round
            .last_sent
            .is_some_and(|sent| Instant::now() < sent + CAP);
        if round.asking.is_none() && !capped {
            self.send(round);
        } else {
            round.held_back = true;
        }
    }

    fn send(&self, round: &mut Round) {
        round.last_sent = Some(Instant::now());
        round.held_back = false;
        round.asking = Some((self.asker.ask(), self.screens.snapshot()));
    }

    fn answered(&self, round: &mut Round, answer: Result<Global, Failure>) {
        let stamp = (self.clock)();
        let Some((_, asked)) = round.asking.take() else {
            return;
        };
        round.set_reason = true;
        match answer {
            Ok(global) => {
                self.reason.set(Ok(()));
                if let Some(held) = self.place(global, asked, stamp) {
                    round.held.push_back(held);
                }
            }
            Err(failure) => self.reason.set(Err(failure.reason())),
        }
    }

    /// `global`, answered at `stamp` to an ask sent on `asked`, as a
    /// position on that layout; `None` when the layout changed since, was
    /// unknown or unsettled, or the point lies outside its box.
    fn place(&self, global: Global, asked: Snapshot, stamp: Duration) -> Option<Held> {
        let changes = asked.changes?;
        if self.screens.changes() != Some(changes) {
            return None;
        }
        let layout = asked.layout.ok()?;
        let x = i64::from(global.x).checked_sub(asked.corner.x)?;
        let y = i64::from(global.y).checked_sub(asked.corner.y)?;
        let point = Point {
            x: u32::try_from(x).ok()?,
            y: u32::try_from(y).ok()?,
        };
        Some(Held {
            until: Instant::now() + HOLD,
            stamp,
            point,
            layout,
            changes,
        })
    }

    fn feed_held(&self, round: &mut Round) {
        let Some(held) = round.held.pop_front() else {
            return;
        };
        if self.screens.changes() == Some(held.changes) {
            self.feed.with(|recorder| {
                let on = Some(&held.layout);
                recorder.position(
                    held.stamp,
                    held.point,
                    Taken {
                        by: PointerSource::Hyprland,
                        on,
                    },
                );
            });
        }
    }
}

impl Round {
    /// When to ask for motion held back: once nothing is in flight and the
    /// cap allows.
    fn next_ask(&self) -> Option<Instant> {
        let free = self.held_back && self.asking.is_none();
        free.then(|| self.last_sent.map_or_else(Instant::now, |sent| sent + CAP))
    }
}

/// The answer to the ask in flight; never, without one.
async fn asked(asking: &mut Option<(Asking, Snapshot)>) -> Result<Global, Failure> {
    match asking {
        Some((ask, _)) => ask.await,
        None => pending().await,
    }
}

/// Sleeps until `deadline`; forever without one.
async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => pending().await,
    }
}

#[cfg(test)]
mod tests;
