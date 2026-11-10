//! How the controller and the input reader share a recording in progress.
//!
//! The controller alone starts and stops recordings: it puts a [`Recorder`]
//! in the [`RecordingSlot`] and takes it out again. The input reader holds a
//! [`RecordingFeed`] and only hands what it reads to the recorder that is
//! there, if any; it never starts or ends one. Both run on the daemon's one
//! thread, so the slot is a plain shared cell, and a stop takes the recorder
//! out at once, with no message to wait for.
//!
//! Each start and stop also bumps a counter the reader watches, so it opens
//! the devices a recording reads (mice) only while one runs, and reads what
//! every device holds when one starts.

use std::cell::RefCell;
use std::rc::Rc;

use tokio::sync::watch;

use super::Recorder;

type Shared = Rc<RefCell<Option<Recorder>>>;

/// The controller's side: where recordings start and stop.
pub struct RecordingSlot {
    recorder: Shared,
    changes: watch::Sender<u64>,
}

/// The input reader's side: what it feeds while a recording runs.
#[derive(Clone)]
pub struct RecordingFeed {
    recorder: Shared,
    changes: watch::Receiver<u64>,
}

impl Default for RecordingSlot {
    fn default() -> Self {
        RecordingSlot::new()
    }
}

impl RecordingSlot {
    /// An empty slot.
    pub fn new() -> RecordingSlot {
        RecordingSlot {
            recorder: Rc::default(),
            changes: watch::Sender::new(0),
        }
    }

    /// A feed for an input reader.
    pub fn feed(&self) -> RecordingFeed {
        RecordingFeed {
            recorder: Rc::clone(&self.recorder),
            changes: self.changes.subscribe(),
        }
    }

    /// Starts recording into `recorder`.
    pub fn start(&self, recorder: Recorder) {
        self.recorder.replace(Some(recorder));
        self.changed();
    }

    /// Stops recording; the recorder, if one ran.
    pub fn take(&self) -> Option<Recorder> {
        let taken = self.recorder.take();
        if taken.is_some() {
            self.changed();
        }
        taken
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.borrow().is_some()
    }

    fn changed(&self) {
        self.changes
            .send_modify(|count| *count = count.wrapping_add(1));
    }
}

impl RecordingFeed {
    /// Whether a recording runs.
    pub fn recording(&self) -> bool {
        self.recorder.borrow().is_some()
    }

    /// Hands something to the recorder, if a recording runs.
    pub fn with(&self, feed: impl FnOnce(&mut Recorder)) {
        if let Some(recorder) = self.recorder.borrow_mut().as_mut() {
            feed(recorder);
        }
    }

    /// Waits until a recording started or stopped since this was last
    /// called (or since the feed was made). Never resolves once the slot is
    /// gone.
    pub async fn changed(&mut self) {
        if self.changes.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;
    use crate::hotkeys::Bindings;
    use crate::recording::tests::created;

    fn recorder() -> Recorder {
        Recorder::new(Bindings::default(), Duration::ZERO, created(), None)
    }

    #[tokio::test]
    async fn the_feed_reaches_only_a_running_recorder_and_sees_each_change() {
        let slot = RecordingSlot::new();
        let mut feed = slot.feed();
        let mut fed = 0;
        feed.with(|_| fed += 1);
        assert!(!feed.recording());

        slot.start(recorder());
        timeout(Duration::from_secs(5), feed.changed())
            .await
            .unwrap();
        feed.with(|_| fed += 1);
        assert!(feed.recording() && slot.is_recording());

        assert!(slot.take().is_some());
        assert!(slot.take().is_none());
        assert!(!slot.is_recording());
        timeout(Duration::from_secs(5), feed.changed())
            .await
            .unwrap();
        feed.with(|_| fed += 1);
        assert_eq!((fed, feed.recording()), (1, false));
    }

    #[tokio::test(start_paused = true)]
    async fn a_feed_without_its_slot_waits_forever() {
        let mut feed = RecordingSlot::new().feed();
        let waited = timeout(Duration::from_secs(3600), feed.changed()).await;
        assert!(waited.is_err());
    }
}
