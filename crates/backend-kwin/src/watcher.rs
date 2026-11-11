//! The watcher: polled on the daemon's thread beside the controller, it
//! keeps KWin's scripts as the brief's "KWin scripts" says, hands the
//! reports to the recorder, and keeps the current layout (brief I4).
//!
//! It reacts to four things, never to a timer: a recording starting or
//! stopping, a new owner of `org.kde.KWin`, a report, and the end of its one
//! call to KWin in flight. That call is a branch of its own, so a KWin that
//! is slow to answer never holds up the reports behind it: they reach the
//! recorder within the 20 ms it waits for late ones.
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

use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::recording::{Layout, RecordingFeed};
use futures_util::StreamExt;
use futures_util::stream;
use tokio::sync::{Notify, mpsc};

use crate::receiver::with_trust;
use crate::report::{Body, Report};
use crate::script::Script;
use crate::scripting::{CallError, Owners, Scripting, bounded};
use crate::session::{Session, log_failure};
use crate::token::Token;

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
    Swept,
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
}

/// What the position script needs next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Nothing,
    Load,
    Unload,
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
    layout_due: bool,
    position: Next,
    /// The position script may be loaded: a load was sent and no unload
    /// has succeeded since.
    position_loaded: bool,
    /// The position script's layout report still counts: no layout report
    /// was lost since it was loaded, so it is not older than one that was.
    layout_from_position: bool,
    job: Option<Job>,
    /// The call in flight loads the position script.
    job_loads_position: bool,
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
            layout_due: false,
            position: Next::Nothing,
            position_loaded: false,
            layout_from_position: false,
            job: None,
            job_loads_position: false,
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
        let Watcher { feed, job, .. } = self;
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
        }
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
        self.layout_due = running;
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
        self.layout_due = self.owner.is_some();
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
            Done::Swept | Done::Loaded(_, Ok(())) => {}
            Done::Loaded(script, Err(err)) => {
                log_failure("load", script, &err);
                match script {
                    Script::Layout => {
                        self.layout_token = None;
                        self.set_layout(None);
                    }
                    Script::Position => self.position_token = None,
                }
                self.publish_trust();
            }
            Done::Unloaded(Ok(())) => self.position_loaded = false,
            Done::Unloaded(Err(err)) => log_failure("unload", Script::Position, &err),
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
            Box::pin(async move {
                session.unload_all(&owner).await;
                Done::Swept
            })
        } else if mem::take(&mut self.layout_due) {
            self.load(Script::Layout, owner)
        } else {
            match mem::replace(&mut self.position, Next::Nothing) {
                Next::Load => {
                    loads_position = true;
                    self.load(Script::Position, owner)
                }
                Next::Unload => Box::pin(async move {
                    Done::Unloaded(session.unload(&owner, Script::Position).await)
                }),
                Next::Nothing => return,
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
                self.layout_from_position = true;
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
