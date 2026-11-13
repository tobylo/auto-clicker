//! The watcher: polled on the daemon's thread beside the controller, it
//! loads the position script while a recording runs and hands its reports
//! to the recorder, each position with the layout the script reported last
//! before it (W01 brief, W4). It tells the daemon when a KWin of its display
//! comes back after a restart (W5); the layout itself comes from the
//! Wayland display.
//!
//! It reacts to a recording starting or stopping, a new owner of
//! `org.kde.KWin`, a report, and the end of its one call to KWin in flight.
//! That call is a branch of its own, so a KWin that is slow to answer never
//! holds up the reports behind it: they reach the recorder within the 20 ms
//! it waits for late ones.
//!
//! Its one timer is the back-off after a failed call, which tries it again
//! a few times ([`BACK_OFF`]): an unload while the daemon is idle, a load
//! while its recording runs (a KWin that just restarted does not answer
//! for a moment). It exists only while a call waits to be tried again: with
//! nothing failed, and once the tries are spent, the watcher sets none.
//!
//! Every load gets a new token, made current before the load is sent (the
//! script reports as soon as it runs, before KWin answers), and a report
//! counts only with a current token. A recording that stops and another
//! that starts before the watcher looks are one wake, which loads the
//! position script again with a new token: the new recording gets its own
//! first position, and nothing the old load sent.

use std::future::{Future, pending, poll_fn};
use std::mem;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::recording::{Layout, RecordingFeed};
use futures_util::StreamExt;
use futures_util::stream;
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, sleep_until};

use crate::display;
use crate::receiver::with_trust;
use crate::report::{Body, Report};
use crate::script::Script;
use crate::scripting::{CallError, Owners, Scripting, bounded};
use crate::session::{Session, log_failure};
use crate::token::Token;

/// How long the watcher waits before each try again of a failed unload or
/// load, counted from the failure before it. A failure after the last is
/// logged; an unload then waits for another call to KWin to succeed, a new
/// owner of KWin, the next recording or the exit, and a load for a new
/// owner or the next recording.
pub(crate) const BACK_OFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// What the watcher hears from the receiver.
pub(crate) struct Inbox {
    pub(crate) reports: mpsc::Receiver<Report>,
    /// A layout report did not fit in the queue: the positions after it
    /// cannot be judged.
    pub(crate) layout_dropped: Arc<Notify>,
}

/// The one call to KWin in flight.
type Job = Pin<Box<dyn Future<Output = Done>>>;

/// How a call to KWin ended.
enum Done {
    Loaded(Result<(), CallError>),
    Unloaded(Result<(), CallError>),
}

/// What woke the watcher.
enum Wake {
    Recording,
    Owner(Option<String>),
    LayoutDropped,
    Report(Report),
    Done(Done),
    BackedOff,
}

/// What the position script needs next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Nothing,
    Load,
    Unload,
    /// Unload once the back-off ends: the last unload failed.
    UnloadAfterBackOff,
    /// Load once the back-off ends: the last load failed while its
    /// recording runs, as when a KWin just restarted does not answer yet.
    LoadAfterBackOff,
}

impl Next {
    /// A call waits for the back-off.
    fn waits(self) -> bool {
        matches!(self, Next::UnloadAfterBackOff | Next::LoadAfterBackOff)
    }
}

pub(crate) struct Watcher<S> {
    session: Rc<Session<S>>,
    feed: RecordingFeed,
    /// Fired each time a KWin of the display comes to own `org.kde.KWin`,
    /// the first one seen included: the display's connection may be to one
    /// that went before, or may have failed while it started.
    restarted: LayoutSignal,
    rng: fastrand::Rng,
    owner: Option<String>,
    position_token: Option<Token>,
    /// The layout the current load reported last, which the positions after
    /// it were taken on; `None` until it reports one, and after a report
    /// that was no layout or was lost.
    taken_on: Option<Layout>,
    /// The current load's first layout report was read.
    first_layout_read: bool,
    /// Every name is unloaded first: what a crashed daemon left, of this
    /// version or an older one.
    sweep_due: bool,
    position: Next,
    /// The position script may be loaded: a load was sent, or a crashed
    /// daemon's may still run, and no unload has succeeded since.
    position_loaded: bool,
    job: Option<Job>,
    /// The call in flight loads the position script.
    job_loads_position: bool,
    /// When the back-off ends; set exactly while an unload waits for it.
    retry_at: Option<Instant>,
    /// Unloads that failed in a row, since a call succeeded or KWin got a
    /// new owner.
    failures: usize,
}

impl<S: Scripting + 'static> Watcher<S> {
    pub(crate) fn new(
        session: Rc<Session<S>>,
        feed: RecordingFeed,
        restarted: LayoutSignal,
    ) -> Self {
        Watcher {
            session,
            feed,
            restarted,
            rng: fastrand::Rng::new(),
            owner: None,
            position_token: None,
            taken_on: None,
            first_layout_read: false,
            sweep_due: false,
            position: Next::Nothing,
            position_loaded: false,
            job: None,
            job_loads_position: false,
            retry_at: None,
            failures: 0,
        }
    }

    /// The same, drawing tokens from `rng`.
    #[cfg(test)]
    pub(crate) fn with_rng(self, rng: fastrand::Rng) -> Self {
        Watcher { rng, ..self }
    }

    /// Runs until dropped.
    pub(crate) async fn run(mut self, mut inbox: Inbox) {
        // Files a crashed daemon left; nothing it loaded is in a KWin that
        // is not running, and a running one is swept below.
        self.session.sweep(None);
        let mut owners = self.follow_owner().await;
        loop {
            self.next_job();
            let wake = self.next_wake(&mut owners, &mut inbox).await;
            self.handle(wake);
        }
    }

    /// Starts following KWin's owner, each owner counted only when it runs
    /// the daemon's display; a bus that cannot tell leaves KWin unknown for
    /// good, so nothing is ever loaded. A new owner during the first
    /// owner's check ends that check: the new one is checked instead.
    async fn follow_owner(&mut self) -> Owners {
        match bounded(self.session.scripting.follow_owner()).await {
            Ok((owner, owners)) => {
                let mut owners = display::each_checked(Rc::clone(&self.session), owners);
                let owner = tokio::select! {
                    biased;
                    Some(newer) = owners.next() => newer,
                    owner = display::checked(&self.session, owner) => owner,
                };
                self.new_owner(owner);
                self.sweep_due = self.owner.is_some();
                owners
            }
            Err(err) => {
                eprintln!(
                    "auto-clicker: cannot follow KWin, so recordings get no positions: {}",
                    err.0
                );
                Box::pin(stream::pending())
            }
        }
    }

    /// Branches in order: a recording's start or stop and a new owner
    /// before the reports, so none is read under a token that just ended.
    async fn next_wake(&mut self, owners: &mut Owners, inbox: &mut Inbox) -> Wake {
        let Watcher {
            feed,
            job,
            retry_at,
            ..
        } = self;
        tokio::select! {
            biased;
            () = feed.changed() => Wake::Recording,
            owner = owners.next() => match owner {
                Some(owner) => Wake::Owner(owner),
                // The connection closed: no owner is ever known again, and
                // the stream is not polled again.
                None => {
                    *owners = Box::pin(stream::pending());
                    Wake::Owner(None)
                }
            },
            () = back_off_end(*retry_at) => Wake::BackedOff,
            () = inbox.layout_dropped.notified() => Wake::LayoutDropped,
            report = received(&mut inbox.reports) => Wake::Report(report),
            done = job_end(job) => Wake::Done(done),
        }
    }

    fn handle(&mut self, wake: Wake) {
        match wake {
            Wake::Recording => self.recording_changed(),
            Wake::Owner(owner) => self.new_owner(owner),
            Wake::LayoutDropped => self.layout_dropped(),
            Wake::Report(report) => self.report(report),
            Wake::Done(done) => self.done(done),
            Wake::BackedOff => self.backed_off(),
        }
        self.drop_unneeded_back_off();
    }

    /// Any change is a new recording or none: the old load's token ends
    /// either way, and a load for it still in flight ends at once. Whatever
    /// that load got to is taken away by the unload that follows, or by the
    /// next load, which unloads first.
    fn recording_changed(&mut self) {
        if mem::take(&mut self.job_loads_position) {
            self.job = None;
        }
        self.position_token = None;
        self.taken_on = None;
        self.position = if self.feed.recording() {
            Next::Load
        } else if self.position_loaded {
            Next::Unload
        } else {
            Next::Nothing
        };
        self.publish_trust();
    }

    /// A new owner: once it is a KWin of the display, the daemon is told,
    /// so the layout's connection to the display is made again (W5). The
    /// first one counts too: the connection the daemon made before it
    /// followed KWin may be to a KWin gone since, or may have timed out.
    fn new_owner(&mut self, owner: Option<String>) {
        let came = owner.is_some() && owner != self.owner;
        self.owner_changed(owner);
        if came {
            self.restarted.changed();
        }
    }

    /// A new owner of `org.kde.KWin` (or none) has none of the scripts: the
    /// call in flight went to the old one, and the position script's token
    /// ends.
    fn owner_changed(&mut self, owner: Option<String>) {
        if owner == self.owner {
            return;
        }
        self.job = None;
        self.owner = owner;
        self.session.owner.replace(self.owner.clone());
        self.position_token = None;
        self.taken_on = None;
        let running = self.owner.is_some();
        self.sweep_due = false;
        self.failures = 0;
        self.position = if running && self.feed.recording() {
            Next::Load
        } else {
            Next::Nothing
        };
        self.position_loaded = false;
        self.session.sweep(None);
        self.publish_trust();
    }

    /// A layout report was lost: the positions after it cannot be judged.
    /// The load's token ends at once, so nothing it queued is taken, and a
    /// recording's position script is loaded again, reporting the layout
    /// first.
    fn layout_dropped(&mut self) {
        if self.position_token.is_none() {
            return;
        }
        self.recording_changed();
    }

    fn report(&mut self, report: Report) {
        let Report { time, token, body } = report;
        if Some(&token) != self.position_token.as_ref() {
            return;
        }
        match body {
            Body::Position(point) => {
                let taken_on = self.taken_on.as_ref();
                self.feed
                    .with(|recorder| recorder.position(time, point, taken_on));
            }
            Body::Layout(layout) => {
                if !mem::replace(&mut self.first_layout_read, true) {
                    self.check_first(layout.as_ref());
                }
                self.taken_on = layout;
            }
        }
    }

    /// Says so when the screens KWin reported at the load are not the
    /// recording's, which the display gave: its positions are left out.
    fn check_first(&self, layout: Option<&Layout>) {
        let mut differs = false;
        self.feed.with(|recorder| {
            differs = !layout.is_some_and(|layout| recorder.takes_positions_on(layout));
        });
        if differs {
            eprintln!(
                "auto-clicker: KWin reports other screens than the Wayland display, so this \
                 recording keeps no positions while it does: {layout:?}"
            );
        }
    }

    fn done(&mut self, done: Done) {
        match done {
            Done::Loaded(Ok(())) => self.succeeded(),
            Done::Loaded(Err(err)) => {
                log_failure("load", Script::Position, &err);
                self.position_token = None;
                // Its recording runs: a stop drops a load in flight. A
                // script directory that failed its check stays failed.
                let retried = self.session.dir.is_ok() && self.back_off();
                if retried {
                    self.position = Next::LoadAfterBackOff;
                }
                self.publish_trust();
            }
            Done::Unloaded(Ok(())) => {
                self.position_loaded = false;
                self.succeeded();
            }
            Done::Unloaded(Err(err)) => {
                log_failure("unload", Script::Position, &err);
                // A recording since then loads the script again, which
                // unloads it first.
                if self.position == Next::Nothing && self.back_off() {
                    self.position = Next::UnloadAfterBackOff;
                }
            }
        }
    }

    /// KWin answered: the count starts afresh, and an unload given up on is
    /// due again.
    fn succeeded(&mut self) {
        self.failures = 0;
        let unload_given_up =
            self.position_loaded && !self.feed.recording() && self.position == Next::Nothing;
        if unload_given_up {
            self.position = Next::Unload;
        }
    }

    /// Counts a failure, and starts the back-off before the next try;
    /// false, with the failure logged, once the tries are spent.
    fn back_off(&mut self) -> bool {
        let failures = self.failures;
        self.failures = failures.saturating_add(1);
        let Some(wait) = BACK_OFF.get(failures) else {
            if failures == BACK_OFF.len() {
                eprintln!(
                    "auto-clicker: KWin keeps failing; its scripts are tried again once it \
                     answers or restarts, at the next recording and at exit"
                );
            }
            return false;
        };
        self.retry_at = Instant::now().checked_add(*wait);
        self.retry_at.is_some()
    }

    /// The back-off ended: the unload that waited for it is due now.
    fn backed_off(&mut self) {
        self.retry_at = None;
        self.position = match self.position {
            Next::UnloadAfterBackOff => Next::Unload,
            Next::LoadAfterBackOff => Next::Load,
            next => next,
        };
    }

    /// Ends the back-off once nothing waits for it: a new owner or a
    /// recording's start or stop replaced the call.
    fn drop_unneeded_back_off(&mut self) {
        if !self.position.waits() {
            self.retry_at = None;
        }
    }

    /// Starts the next call to KWin, when none is in flight and KWin runs:
    /// the sweep, then what the position script needs.
    fn next_job(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(owner) = self.owner.clone() else {
            return;
        };
        let session = Rc::clone(&self.session);
        let mut loads_position = false;
        let job: Job = if mem::take(&mut self.sweep_due) {
            // The position unload counts as any other: a crashed daemon's
            // script that stays loaded is tried again after the back-off,
            // or unloaded when a recording that starts meanwhile stops.
            self.position_loaded = true;
            Box::pin(async move {
                let (_, position) = tokio::join!(
                    session.unload_logged(&owner, Script::OldLayout),
                    session.unload(&owner, Script::Position)
                );
                Done::Unloaded(position)
            })
        } else {
            match self.position {
                Next::Load => {
                    self.position = Next::Nothing;
                    loads_position = true;
                    self.load(owner)
                }
                Next::Unload => {
                    self.position = Next::Nothing;
                    Box::pin(async move {
                        Done::Unloaded(session.unload(&owner, Script::Position).await)
                    })
                }
                Next::Nothing | Next::UnloadAfterBackOff | Next::LoadAfterBackOff => return,
            }
        };
        self.job = Some(job);
        self.job_loads_position = loads_position;
    }

    /// A load of the position script with a new token, current from now
    /// on.
    fn load(&mut self, owner: String) -> Job {
        let token = Token::new(&mut self.rng);
        self.position_token = Some(token.clone());
        self.first_layout_read = false;
        self.position_loaded = true;
        self.publish_trust();
        let session = Rc::clone(&self.session);
        Box::pin(async move { Done::Loaded(session.load(&owner, &token).await) })
    }

    /// Tells the receiver whom to accept calls from now.
    fn publish_trust(&self) {
        let tokens = self.position_token.iter().cloned().collect();
        with_trust(&self.session.trust, |trust| {
            trust.owner = self.owner.clone();
            trust.tokens = tokens;
        });
    }
}

/// The next report; never resolves once the receiver is gone.
async fn received(reports: &mut mpsc::Receiver<Report>) -> Report {
    match reports.recv().await {
        Some(report) => report,
        None => pending().await,
    }
}

/// The end of the back-off; never resolves while there is none.
async fn back_off_end(at: Option<Instant>) {
    match at {
        Some(at) => sleep_until(at).await,
        None => pending().await,
    }
}

/// The end of the call in flight; never resolves while there is none.
fn job_end(job: &mut Option<Job>) -> impl Future<Output = Done> + '_ {
    poll_fn(move |cx| {
        let Some(running) = job else {
            return Poll::Pending;
        };
        let done = running.as_mut().poll(cx);
        if done.is_ready() {
            *job = None;
        }
        done
    })
}

#[cfg(test)]
mod tests;
