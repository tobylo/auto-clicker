//! The controller's recordings: starting one from the tray or the hotkey,
//! and stopping and saving it, whatever stops it.
//!
//! A recording runs in the [`RecordingSlot`](crate::recording::RecordingSlot)
//! the input reader feeds. Every stop takes the recorder out of it in the
//! same step, finishes it and saves it before the controller handles
//! anything else, so a lock, sleep, the time limit or quitting saves what
//! was recorded up to then.

use tokio::time::Instant;

use super::{Controller, Platform, State};
use crate::hotkeys::{Binding, HotkeyStatus, Unavailable};
use crate::recording::{Created, Ending, MAX_DURATION, Recorder, Recording, save};

/// How a recording was stopped, which decides how it is trimmed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StoppedBy {
    /// The tray menu: the trip to the tray is trimmed.
    Tray,
    /// The hotkey (the recorder trims it itself), the session, the time
    /// limit or quitting.
    Other,
}

impl<P: Platform> Controller<P> {
    /// The recording hotkey: starts recording at once, cancels a tray
    /// countdown, or stops and saves the recording.
    pub(super) fn toggle_recording(&mut self) {
        match self.state {
            State::Idle => {
                if self.may_record() {
                    self.begin_recording(Some(self.config.hotkeys.toggle_recording));
                }
            }
            State::RecordCountdown(_) => self.state = State::Idle,
            State::Recording(_) => self.finish_recording(StoppedBy::Other, None),
            State::Countdown(_) | State::Release(_) | State::Clicking(_) => {
                self.notice = Some("Not recording: clicking is on; stop it first".to_owned());
            }
        }
    }

    /// The tray's Record: counts down, then records.
    pub(super) fn record_from_tray(&mut self) {
        if !matches!(self.state, State::Idle) || !self.may_record() {
            return;
        }
        let delay = self.config.tray_start_delay;
        if delay.is_zero() {
            self.begin_recording(None);
        } else if let Some(deadline) = Instant::now().checked_add(delay) {
            self.state = State::RecordCountdown(deadline);
        }
    }

    /// The tray's Stop recording: stops and saves without the trip to the
    /// tray, or cancels the countdown.
    pub(super) fn stop_recording_from_tray(&mut self) {
        match self.state {
            State::RecordCountdown(_) => self.state = State::Idle,
            State::Recording(_) => self.finish_recording(StoppedBy::Tray, None),
            _ => {}
        }
    }

    /// Notes when the tray menu opened, for trimming a stop from it. A
    /// recording starts with none noted.
    pub(super) fn menu_opened(&mut self) {
        self.menu_opened = Some(self.platform.event_clock());
    }

    /// Stops and saves a recording, saying why when `why` is given;
    /// anything else is left alone.
    pub(super) fn stop_recording(&mut self, why: Option<String>) {
        if matches!(self.state, State::Recording(_)) {
            self.finish_recording(StoppedBy::Other, why);
        }
    }

    /// The input reader ended: a recording has nothing left to read, so it
    /// stops and saves what it has, and a countdown to one is cancelled.
    pub(super) fn input_lost(&mut self, why: String) {
        match self.state {
            State::RecordCountdown(_) => {
                self.state = State::Idle;
                self.notice = Some(format!("Not recording: {why}"));
            }
            State::Recording(_) => {
                self.finish_recording(StoppedBy::Other, Some(format!("Recording stopped: {why}")));
            }
            _ => {}
        }
    }

    pub(super) fn recording_limit_reached(&mut self) {
        let minutes = MAX_DURATION.as_secs() / 60;
        let why = format!("Recording stopped at its {minutes}-minute limit");
        self.finish_recording(StoppedBy::Other, Some(why));
    }

    /// Whether a recording may start; says why not when it may not.
    fn may_record(&mut self) -> bool {
        if !self.session_allows("Not recording") {
            return false;
        }
        match &self.hotkey_status {
            Some(HotkeyStatus::Unavailable(
                why @ (Unavailable::NoPermission | Unavailable::WatchFailed(_)),
            )) => {
                self.notice = Some(format!("Not recording: {why}"));
                false
            }
            _ => true,
        }
    }

    /// Starts recording now. `started_by` is the hotkey that started it,
    /// whose keys are still down.
    pub(super) fn begin_recording(&mut self, started_by: Option<Binding>) {
        self.state = State::Idle;
        let Some(created) = self.platform.local_time().and_then(Created::new) else {
            self.notice = Some("Not recording: the clock could not be read".to_owned());
            return;
        };
        let Some(limit) = Instant::now().checked_add(MAX_DURATION) else {
            return;
        };
        let start = self.platform.event_clock();
        let recorder = Recorder::new(self.config.hotkeys, start, created, started_by);
        self.recording.start(recorder);
        self.menu_opened = None;
        self.notice = None;
        self.state = State::Recording(limit);
    }

    /// Takes the recording from the reader, finishes and saves it, and says
    /// how that went, after `why`.
    fn finish_recording(&mut self, stopped_by: StoppedBy, why: Option<String>) {
        self.state = State::Idle;
        let menu_opened = self.menu_opened.take();
        let Some(recorder) = self.recording.take() else {
            return;
        };
        let at = self.platform.event_clock();
        let ending = match stopped_by {
            StoppedBy::Tray => Ending::Tray { at, menu_opened },
            StoppedBy::Other => Ending::At(at),
        };
        let outcome = match recorder.finish(ending) {
            Ok(Some(recording)) => self.save(&recording),
            Ok(None) => "Nothing was recorded, so nothing was saved".to_owned(),
            Err(err) => failed(&err.to_string()),
        };
        self.notice = Some(match why {
            Some(why) => format!("{why}. {outcome}"),
            None => outcome,
        });
    }

    /// Saves `recording`; what the tray says about it.
    fn save(&self, recording: &Recording) -> String {
        let Some(dir) = self.platform.recordings_dir() else {
            return failed("no data directory: neither XDG_DATA_HOME nor HOME is set");
        };
        match save(&dir, recording) {
            Ok(_) => format!("Saved \u{201c}{}\u{201d}", recording.name()),
            Err(err) => failed(&err.to_string()),
        }
    }
}

/// A recording that could not be saved: logged, and shown.
fn failed(why: &str) -> String {
    let message = format!("The recording could not be saved: {why}");
    eprintln!("auto-clicker: {message}");
    message
}
