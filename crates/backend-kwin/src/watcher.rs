//! The watcher: polled on the daemon's thread beside the controller, it
//! keeps KWin's scripts as the brief's "KWin scripts" says, hands the
//! reports to the recorder, and keeps the current layout (brief I4).
//!
//! It reacts to a recording starting or stopping, a new owner of
//! `org.kde.KWin`, a report, and the end of its one call to KWin in flight.
//! That call is a branch of its own, so a KWin that is slow to answer never
//! holds up the reports behind it: they reach the recorder within the 20 ms
//! it waits for late ones.
//!
//! Its one timer is the back-off after a failed layout load or position
//! unload, which tries the call again a few times while the daemon is idle
//! ([`BACK_OFF`]). It exists only while such a call waits to be tried
//! again: with nothing failed, and once the tries are spent, the watcher
//! sets none.
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

use crate::receiver::with_trust;
use crate::report::{Body, Report};
use crate::script::Script;
use crate::scripting::{CallError, Owners, Scripting, bounded};
use crate::session::{Session, log_failure};
use crate::token::Token;

/// How long the watcher waits before each try again of a failed layout
/// load or position unload, counted from the failure before it. A failure
/// after the last is logged, and the call waits for another call to KWin
/// to succeed, a new owner of KWin, the next recording or the exit.
const BACK_OFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// What the watcher hears from the receiver.
pub(crate) struct Inbox {
    pub(crate) reports: mpsc::Receiver<Report>,
    /// A layout report did not fit in the queue.
    pub(crate) layout_dropped: Arc<Notify>,
}

/// The one call to KWin in flight.
type Job = Pin<Box<dyn Future<Output = Done>>>;

/// How a call to KWin ended.
enum Done {
    Loaded(Script, Result<(), CallError>),
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

/// Whether the layout script is to be loaded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Due {
    No,
    Now,
    /// Once the back-off ends: its last load failed.
    AfterBackOff,
}

/// What the position script needs next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Nothing,
    Load,
    Unload,
    /// Unload once the back-off ends: the last unload failed.
    UnloadAfterBackOff,
}

pub(crate) struct Watcher<S> {
    session: Rc<Session<S>>,
    feed: RecordingFeed,
    changed: LayoutSignal,
    rng: fastrand::Rng,
    owner: Option<String>,
    layout_token: Option<Token>,
    position_token: Option<Token>,
    /// Both names are unloaded first: what a crashed daemon left.
    sweep_due: bool,
    layout_due: Due,
    position: Next,
    /// The position script may be loaded: a load was sent, or a crashed
    /// daemon's may still run, and no unload has succeeded since.
    position_loaded: bool,
    /// The position script's layout report still counts: no layout report
    /// was lost since it was loaded, so it is not older than one that was.
    layout_from_position: bool,
    job: Option<Job>,
    /// The call in flight loads the position script.
    job_loads_position: bool,
    /// When the back-off ends; set exactly while a call waits for it.
    retry_at: Option<Instant>,
    /// Layout loads and position unloads that failed in a row, since a call
    /// succeeded or KWin got a new owner.
    failures: usize,
}

impl<S: Scripting + 'static> Watcher<S> {
    pub(crate) fn new(session: Rc<Session<S>>, feed: RecordingFeed, changed: LayoutSignal) -> Self {
        Watcher {
            session,
            feed,
            changed,
            rng: fastrand::Rng::new(),
            owner: None,
            layout_token: None,
            position_token: None,
            sweep_due: false,
            layout_due: Due::No,
            position: Next::Nothing,
            position_loaded: false,
            layout_from_position: false,
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

    /// Starts following KWin's owner; a bus that cannot tell leaves KWin
    /// unknown for good, so nothing is ever loaded.
    async fn follow_owner(&mut self) -> Owners {
        match bounded(self.session.scripting.follow_owner()).await {
            Ok((owner, owners)) => {
                self.owner_changed(owner);
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
            Wake::Owner(owner) => self.owner_changed(owner),
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
        if self.feed.recording() && self.layout_given_up() {
            self.layout_due = Due::Now;
        }
        self.position = if self.feed.recording() {
            Next::Load
        } else if self.position_loaded {
            Next::Unload
        } else {
            Next::Nothing
        };
        self.publish_trust();
    }

    /// A new owner of `org.kde.KWin` (or none) has none of the scripts: the
    /// call in flight went to the old one, every token ends and the layout
    /// is unknown until the new layout script reports.
    fn owner_changed(&mut self, owner: Option<String>) {
        if owner == self.owner {
            return;
        }
        self.job = None;
        self.owner = owner;
        self.session.owner.replace(self.owner.clone());
        self.layout_token = None;
        self.position_token = None;
        self.set_layout(None);
        let running = self.owner.is_some();
        self.sweep_due = false;
        self.layout_due = if running { Due::Now } else { Due::No };
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

    /// A layout report was lost: the layout is unknown until the layout
    /// script, loaded again, reports it. The old load's token ends at once,
    /// so what it queued before the loss cannot bring an older layout back
    /// while a call to KWin holds up the new load.
    fn layout_dropped(&mut self) {
        self.layout_token = None;
        self.layout_from_position = false;
        self.set_layout(None);
        if self.owner.is_some() {
            self.layout_due = Due::Now;
        }
        self.publish_trust();
    }

    fn report(&mut self, report: Report) {
        let Report { time, token, body } = report;
        match body {
            Body::Position(point) if Some(&token) == self.position_token.as_ref() => {
                let current = self.session.layout.borrow();
                self.feed
                    .with(|recorder| recorder.position(time, point, current.as_ref()));
            }
            Body::Layout(layout) if self.takes_layout_from(&token) => self.set_layout(layout),
            Body::Position(_) | Body::Layout(_) => {}
        }
    }

    fn done(&mut self, done: Done) {
        match done {
            Done::Loaded(_, Ok(())) => self.succeeded(),
            Done::Loaded(script, Err(err)) => {
                log_failure("load", script, &err);
                match script {
                    Script::Layout => {
                        self.layout_token = None;
                        self.set_layout(None);
                        // A script directory that failed its check stays
                        // failed: no load is tried until the next start.
                        if self.session.dir.is_ok() && self.back_off() {
                            self.layout_due = Due::AfterBackOff;
                        }
                    }
                    Script::Position => self.position_token = None,
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

    /// KWin answered: the count starts afresh, and a call given up on is
    /// due again, since the other's failures may have spent its tries.
    fn succeeded(&mut self) {
        self.failures = 0;
        if self.layout_given_up() {
            self.layout_due = Due::Now;
        }
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
        // The earlier deadline wins: a call waiting already is never put
        // off by another's failure, only tried sooner.
        let Some(at) = Instant::now().checked_add(*wait) else {
            return false;
        };
        self.retry_at = Some(self.retry_at.map_or(at, |old| old.min(at)));
        true
    }

    /// The layout load failed after its last back-off: KWin runs, no layout
    /// script is loaded or being loaded, and none is due. The script
    /// directory is fine, or no load could succeed.
    fn layout_given_up(&self) -> bool {
        self.owner.is_some()
            && self.layout_token.is_none()
            && self.layout_due == Due::No
            && self.session.dir.is_ok()
    }

    /// The back-off ended: what waited for it is due now.
    fn backed_off(&mut self) {
        self.retry_at = None;
        if self.layout_due == Due::AfterBackOff {
            self.layout_due = Due::Now;
        }
        if self.position == Next::UnloadAfterBackOff {
            self.position = Next::Unload;
        }
    }

    /// Ends the back-off once nothing waits for it: a new owner, a lost
    /// layout report or a recording made the call due at once, or
    /// replaced it.
    fn drop_unneeded_back_off(&mut self) {
        let waits =
            self.layout_due == Due::AfterBackOff || self.position == Next::UnloadAfterBackOff;
        if !waits {
            self.retry_at = None;
        }
    }

    /// Starts the next call to KWin, when none is in flight and KWin runs:
    /// the sweep, then the layout script, then what the position script
    /// needs.
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
                    session.unload_logged(&owner, Script::Layout),
                    session.unload(&owner, Script::Position)
                );
                Done::Unloaded(position)
            })
        } else if self.layout_due == Due::Now {
            self.layout_due = Due::No;
            self.load(Script::Layout, owner)
        } else {
            match self.position {
                Next::Load => {
                    self.position = Next::Nothing;
                    loads_position = true;
                    self.load(Script::Position, owner)
                }
                Next::Unload => {
                    self.position = Next::Nothing;
                    Box::pin(async move {
                        Done::Unloaded(session.unload(&owner, Script::Position).await)
                    })
                }
                Next::Nothing | Next::UnloadAfterBackOff => return,
            }
        };
        self.job = Some(job);
        self.job_loads_position = loads_position;
    }

    /// A load of `script` with a new token, current from now on.
    fn load(&mut self, script: Script, owner: String) -> Job {
        let token = Token::new(&mut self.rng);
        match script {
            Script::Layout => self.layout_token = Some(token.clone()),
            Script::Position => {
                self.position_token = Some(token.clone());
                self.position_loaded = true;
                // A layout script that was loaded: one only being loaded
                // after a back-off may never run to see a change.
                self.layout_from_position = self.layout_token.is_some();
            }
        }
        self.publish_trust();
        let session = Rc::clone(&self.session);
        Box::pin(async move { Done::Loaded(script, session.load(&owner, script, &token).await) })
    }

    /// Whether a layout report with `token` is taken: from the layout
    /// script, or from the position script while a layout script runs and
    /// no layout report was lost since the position script was loaded. A
    /// layout only the position script reported would stay after the
    /// recording with nothing to see it change; and one it queued before a
    /// lost report is older than what was lost.
    fn takes_layout_from(&self, token: &Token) -> bool {
        let Some(layout) = &self.layout_token else {
            return false;
        };
        let from_position =
            self.layout_from_position && self.position_token.as_ref() == Some(token);
        token == layout || from_position
    }

    /// Tells the receiver whom to accept calls from now.
    fn publish_trust(&self) {
        let tokens = [&self.layout_token, &self.position_token]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        with_trust(&self.session.trust, |trust| {
            trust.owner = self.owner.clone();
            trust.tokens = tokens;
        });
    }

    /// Makes `layout` the current one, and tells the controller when that
    /// changed it.
    fn set_layout(&self, layout: Option<Layout>) {
        let old = self.session.layout.replace(layout);
        if *self.session.layout.borrow() != old {
            self.changed.changed();
        }
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
