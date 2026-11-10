//! Sessions one after another: when a new one is opened, and when the
//! reader gives up.

use super::*;

/// What one scripted session does.
#[derive(Clone, Copy)]
enum Plan {
    /// Ends with this reason before it has bound the shortcuts.
    Fails(&'static str),
    /// Binds, says hotkeys work, then ends with this reason.
    BindsThenEnds(&'static str),
    /// Binds, says hotkeys work, and runs until dropped.
    Serves,
}

/// A scripted desktop: each session notes that it started and follows its
/// plan; sessions past the end of the script serve.
struct Script {
    sessions: Vec<Plan>,
    started: Rc<RefCell<usize>>,
}

impl Desktop for Script {
    async fn session(&mut self, send: &mut dyn FnMut(HotkeyEvent)) -> String {
        let n = {
            let mut started = self.started.borrow_mut();
            *started += 1;
            *started - 1
        };
        match self.sessions.get(n).copied().unwrap_or(Plan::Serves) {
            Plan::Fails(why) => why.to_owned(),
            Plan::BindsThenEnds(why) => {
                send(AVAILABLE);
                why.to_owned()
            }
            Plan::Serves => {
                send(AVAILABLE);
                pending().await
            }
        }
    }
}

/// A running reader: owner changes go in, events come out.
struct Reader {
    owners: Option<mpsc::UnboundedSender<bool>>,
    events: Rc<RefCell<Vec<HotkeyEvent>>>,
    started: Rc<RefCell<usize>>,
    task: tokio::task::JoinHandle<()>,
}

impl Reader {
    async fn start(sessions: Vec<Plan>) -> Reader {
        let (owners, changes) = mpsc::unbounded_channel();
        let events = Rc::new(RefCell::new(Vec::new()));
        let started = Rc::new(RefCell::new(0));
        let mut script = Script {
            sessions,
            started: started.clone(),
        };
        let sink = events.clone();
        let task = tokio::task::spawn_local(async move {
            let changes = futures_util::stream::unfold(changes, |mut changes| async {
                changes.recv().await.map(|change| (change, changes))
            });
            run(&mut script, changes.boxed_local(), |event| {
                sink.borrow_mut().push(event);
            })
            .await;
        });
        settle().await;
        Reader {
            owners: Some(owners),
            events,
            started,
            task,
        }
    }

    async fn owner(&self, present: bool) {
        self.owners.as_ref().unwrap().send(present).unwrap();
        settle().await;
    }

    fn take(&self) -> Vec<HotkeyEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    fn sessions(&self) -> usize {
        *self.started.borrow()
    }
}

const AVAILABLE: HotkeyEvent = HotkeyEvent::Status(HotkeyStatus::Available);

#[tokio::test(start_paused = true)]
async fn a_portal_service_that_stops_or_restarts_is_reported_and_bound_again_at_once() {
    local(async {
        for (present, why) in [
            (false, "a portal service of the desktop stopped"),
            (true, "a portal service of the desktop restarted"),
        ] {
            let r = Reader::start(Vec::new()).await;
            assert_eq!(r.take(), [AVAILABLE]);
            let start = Instant::now();
            r.owner(present).await;
            // The new session starts the service if it is gone.
            assert_eq!((r.sessions(), r.take()), (2, vec![lost(why), AVAILABLE]));
            assert_eq!(start.elapsed(), Duration::ZERO);
            assert!(!r.task.is_finished());
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_that_fails_is_tried_again_only_when_a_portal_service_changes() {
    local(async {
        let script = vec![Plan::Fails("declined"), Plan::BindsThenEnds("closed")];
        let r = Reader::start(script).await;
        assert_eq!(r.take(), [lost("declined")]);
        // No retry by itself, however long.
        advance(Duration::from_secs(24 * 3600)).await;
        settle().await;
        assert_eq!((r.sessions(), r.take()), (1, Vec::new()));
        r.owner(true).await;
        assert_eq!(r.sessions(), 2);
        assert_eq!(
            r.take(),
            [
                lost("a portal service of the desktop restarted"),
                AVAILABLE,
                lost("closed")
            ]
        );
        advance(Duration::from_secs(24 * 3600)).await;
        settle().await;
        assert_eq!(r.sessions(), 2);
        r.owner(false).await;
        assert_eq!(r.sessions(), 3);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_service_that_dies_on_every_bind_is_not_started_for_ever() {
    local(async {
        // Each session starts the service, which dies before the bind is
        // done: the reader hears it stop, and would start it again.
        let r = Reader::start(vec![Plan::Fails("CreateSession: the service died"); 64]).await;
        for _ in 0..64 {
            if r.task.is_finished() {
                break;
            }
            r.owner(false).await;
        }
        assert_eq!(r.sessions(), usize::try_from(MAX_FAILED_IN_A_ROW).unwrap());
        assert!(r.task.is_finished());
        let events = r.take();
        let Some(HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(last)))) =
            events.last()
        else {
            panic!("{events:?}");
        };
        assert!(
            last.contains("could not be bound in 5 tries") && last.contains("restart auto-clicker"),
            "{last}"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_fifth_failure_ends_the_reader_without_waiting_for_another_change() {
    local(async {
        let r = Reader::start(vec![Plan::Fails("no"); 8]).await;
        // Four changes open sessions two to five; the fifth fails too.
        for _ in 0..4 {
            r.owner(true).await;
        }
        assert_eq!(r.sessions(), 5);
        assert!(
            r.task.is_finished(),
            "it waits for a change it will not act on"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn sessions_cut_short_by_a_service_change_count_as_failed_too() {
    local(async {
        // Sessions that never get as far as binding, each ended by the
        // service changing hands again.
        struct NeverBinds(Rc<RefCell<usize>>);
        impl Desktop for NeverBinds {
            async fn session(&mut self, _send: &mut dyn FnMut(HotkeyEvent)) -> String {
                *self.0.borrow_mut() += 1;
                pending().await
            }
        }
        let started = Rc::new(RefCell::new(0));
        let mut desktop = NeverBinds(started.clone());
        let changes = futures_util::stream::iter(std::iter::repeat_n(true, 64));
        let mut events = Vec::new();
        run(&mut desktop, changes, |event| events.push(event)).await;
        assert_eq!(*started.borrow(), 0, "the changes were all queued first");
        assert_eq!(events.len(), usize::try_from(MAX_FAILED_IN_A_ROW).unwrap());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_that_bound_starts_the_count_again() {
    local(async {
        // Four failures, a session that binds, four more: never five in a
        // row, so the reader keeps going.
        let mut script = vec![Plan::Fails("no"); 4];
        script.push(Plan::BindsThenEnds("closed"));
        script.extend([Plan::Fails("no"); 4]);
        let r = Reader::start(script).await;
        for _ in 0..9 {
            r.owner(true).await;
        }
        assert_eq!(r.sessions(), 10);
        assert!(!r.task.is_finished());
        assert_eq!(r.take().last(), Some(&AVAILABLE));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_reader_ends_when_the_bus_can_no_longer_be_watched() {
    local(async {
        // While a session runs, and while waiting for the portal.
        for sessions in [Vec::new(), vec![Plan::Fails("failed")]] {
            let mut r = Reader::start(sessions).await;
            r.take();
            r.owners = None;
            settle().await;
            assert!(r.task.is_finished());
            let events = r.take();
            assert!(
                events.len() <= 1,
                "one last report at most, then nothing: {events:?}"
            );
        }
        let mut r = Reader::start(Vec::new()).await;
        r.take();
        r.owners = None;
        settle().await;
        let events = r.take();
        assert!(
            matches!(
                events.as_slice(),
                [HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(why)))]
                    if why.contains("can no longer be watched")
            ),
            "{events:?}"
        );
    })
    .await;
}
