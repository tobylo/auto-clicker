//! The controller's recordings: starting one from the tray or the hotkey,
//! and stopping and saving it, whatever stops it.
//!
//! A recording runs in the [`RecordingSlot`](crate::recording::RecordingSlot)
//! the input reader feeds. Every stop takes the recorder out of it in the
//! same step, finishes it and saves it before the controller handles
//! anything else, so a lock, sleep, the time limit or quitting saves what
//! was recorded up to then.

use tokio::time::Instant;

use super::view::NoRecording;
use super::{Controller, InputReader, Platform, State};
use crate::hotkeys::{Binding, HotkeyStatus, Unavailable};
use crate::message::{Hindrance, NoPositions, Notice, Quoted, SaveFailure, Saving, Start, Stop};
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

/// Why no recording can start with the hotkeys read by `reader`, whose last
/// status is `status`: the one answer both the refusal and the tray's Record
/// item follow.
pub(super) fn no_recording(
    reader: InputReader,
    status: Option<&HotkeyStatus>,
) -> Option<NoRecording> {
    if reader == InputReader::Desktop {
        return Some(NoRecording::DesktopHotkeys);
    }
    match status {
        Some(HotkeyStatus::Unavailable(
            why @ (Unavailable::NoPermission | Unavailable::WatchFailed(_)),
        )) => Some(NoRecording::Keyboards(why.clone())),
        _ => None,
    }
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
            State::Countdown(..) | State::Release(..) | State::Opening(_) | State::Running(_) => {
                self.notice = Some(not_recording(Hindrance::ClickingOn));
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
    pub(super) fn stop_recording(&mut self, why: Option<Stop>) {
        if matches!(self.state, State::Recording(_)) {
            self.finish_recording(StoppedBy::Other, why);
        }
    }

    /// The input reader ended: a recording has nothing left to read, so it
    /// stops and saves what it has, and a countdown to one is cancelled.
    pub(super) fn input_lost(&mut self, why: &Unavailable) {
        match self.state {
            State::RecordCountdown(_) => {
                self.state = State::Idle;
                self.notice = Some(not_recording(Hindrance::Hotkeys(why.clone())));
            }
            State::Recording(_) => {
                self.finish_recording(StoppedBy::Other, Some(Stop::Hotkeys(why.clone())));
            }
            _ => {}
        }
    }

    /// The recording filled up: it stops and saves what it holds.
    pub(super) fn recording_full(&mut self) {
        if matches!(self.state, State::Recording(_)) && self.recording.is_full() {
            self.finish_recording(StoppedBy::Other, Some(Stop::Full));
        }
    }

    pub(super) fn recording_limit_reached(&mut self) {
        let minutes = MAX_DURATION.as_secs() / 60;
        self.finish_recording(StoppedBy::Other, Some(Stop::Limit { minutes }));
    }

    /// Whether a recording may start; says why not when it may not.
    fn may_record(&mut self) -> bool {
        if !self.session_allows(Start::Recording) {
            return false;
        }
        match self.no_recording() {
            Some(why) => {
                self.notice = Some(not_recording(Hindrance::NoRecording(why)));
                false
            }
            None => true,
        }
    }

    /// Why no recording can start here, if none can.
    pub(super) fn no_recording(&self) -> Option<NoRecording> {
        no_recording(self.platform.input_reader(), self.hotkey_status.as_ref())
    }

    /// Why a recording would keep movement only, while one can be made at
    /// all: the screens are not known, or nothing reports the pointer.
    pub(super) fn no_positions(&self) -> Option<NoPositions> {
        if self.no_recording().is_some() {
            return None;
        }
        match (self.platform.layout(), self.platform.pointer()) {
            (Err(why), _) => Some(NoPositions::Screens(why)),
            (Ok(_), Err(why)) => Some(NoPositions::Pointer(why)),
            (Ok(_), Ok(())) => None,
        }
    }

    /// Starts recording now. `started_by` is the hotkey that started it,
    /// whose keys are still down.
    pub(super) fn begin_recording(&mut self, started_by: Option<Binding>) {
        self.state = State::Idle;
        let Some(created) = self.platform.local_time().and_then(Created::new) else {
            self.notice = Some(not_recording(Hindrance::NoClock));
            return;
        };
        let Some(limit) = Instant::now().checked_add(MAX_DURATION) else {
            return;
        };
        let start = self.platform.event_clock();
        let mut recorder = Recorder::new(self.config.hotkeys, start, created, started_by);
        if let Ok(layout) = self.platform.layout() {
            recorder = recorder.with_layout(layout);
        }
        self.recording.start(recorder);
        self.menu_opened = None;
        self.notice = None;
        self.state = State::Recording(limit);
    }

    /// Takes the recording from the reader, finishes and saves it, and says
    /// how that went, after `why`.
    fn finish_recording(&mut self, stopped_by: StoppedBy, stop: Option<Stop>) {
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
        let saving = match recorder.finish(ending) {
            Ok(Some(recording)) => self.save(&recording),
            Ok(None) => Saving::Nothing,
            Err(err) => failed(SaveFailure::Invalid(Quoted::new(err.to_string()))),
        };
        self.notice = Some(Notice::Recorded { stop, saving });
    }

    /// Saves `recording`, which becomes the one the playback hotkey plays;
    /// what the tray says about it.
    fn save(&mut self, recording: &Recording) -> Saving {
        let Some(dir) = self.platform.recordings_dir() else {
            return failed(SaveFailure::NoDataDirectory);
        };
        match save(&dir, recording) {
            Ok(path) => Saving::Saved {
                name: Quoted::new(recording.name()),
                file: self.saved(&path),
            },
            Err(err) => failed(SaveFailure::Write(Quoted::new(err.to_string()))),
        }
    }
}

/// The notice that no recording started, and why.
fn not_recording(why: Hindrance) -> Notice {
    Notice::Refused {
        start: Start::Recording,
        why,
    }
}

/// A recording that could not be saved: logged, and shown.
fn failed(why: SaveFailure) -> Saving {
    let saving = Saving::Failed(why);
    eprintln!("auto-clicker: {saving}");
    saving
}
