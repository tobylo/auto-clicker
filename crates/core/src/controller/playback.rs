//! The controller's playback: choosing and loading the recording, and
//! starting it from the tray or the hotkey.
//!
//! A recording is read and checked once, when playing it is asked for and
//! before any wait: what was loaded then is what plays after the countdown
//! or the wait for the hotkey's modifiers. A file that is missing, invalid
//! or too short to repeat says so in a notice, and nothing is pressed.
//!
//! Listing and loading run on the controller's own thread, like saving a
//! recording, and only while idle, when nothing is pressed and nothing waits
//! on a deadline. An hour of movement at a thousand moves a second (107 MB)
//! loads in 0.22 s on the dev box, and the same hour with a position each
//! millisecond (193 MB) in 0.39 s; no recording file is larger than
//! 256 MiB.
//!
//! Whether a recording's positions are used is decided when its run begins,
//! against the layout then, and the tray says why when they are not. A
//! layout change while positions play stops the playback: they would land
//! elsewhere.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use tokio::time::Instant;

use super::desktop::OwnKeys;
use super::{
    Controller, Job, LISTED_RECORDINGS, Listed, PLAY_RELEASE_WAIT, Platform, RunFuture, State,
};
use crate::input::{Held, Input, InputBackend, Key};
use crate::playback::{MIN_LOOP_LENGTH, Motion, Playback, play};
use crate::recording::{Change, FileName, Layout, LoadError, Recording, list, load};
use crate::run::Signals;

/// A recording about to play: loaded, checked, and named for the tray.
pub(super) struct Playing {
    playback: Playback,
    label: String,
    /// Every key it presses.
    keys: Arc<[Key]>,
    /// The hotkey's modifiers were still down when the wait for them ended.
    modifier_held: bool,
}

impl Playing {
    /// The recording, as the tray names it.
    pub(super) fn label(&self) -> &str {
        &self.label
    }

    /// Every key the recording presses.
    pub(super) fn keys(&self) -> Arc<[Key]> {
        Arc::clone(&self.keys)
    }

    /// The same, started although a modifier of the hotkey was still held.
    pub(super) fn modifier_held(self) -> Playing {
        Playing {
            modifier_held: true,
            ..self
        }
    }

    /// What the tray says when it starts on `layout`, the current one, if
    /// anything.
    pub(super) fn start_notice(&self, layout: Option<&Layout>) -> Option<String> {
        let held = self.modifier_held.then(|| {
            format!(
                "Playing although the hotkey was still held after {} s; its keys mix into what plays",
                PLAY_RELEASE_WAIT.as_secs()
            )
        });
        let (_, fallback) = self.playback.motion(layout);
        let fallback = fallback.map(|why| why.notice().to_owned());
        match (held, fallback) {
            (Some(held), Some(fallback)) => Some(format!("{held}. {fallback}")),
            (held, fallback) => held.or(fallback),
        }
    }

    /// The run that plays it on `input`, on `layout`, the current one; and
    /// the layout it moves to positions on, if it does.
    pub(super) fn run<B, S, D>(
        self,
        input: Input<B>,
        layout: Option<&Layout>,
        signals: Signals<S, D>,
    ) -> (RunFuture<B>, Option<Layout>)
    where
        B: InputBackend + 'static,
        S: Future<Output = ()> + 'static,
        D: Future<Output = ()> + 'static,
    {
        let (motion, _) = self.playback.motion(layout);
        let positions = match &motion {
            Motion::Positions(layout) => Some(layout.clone()),
            Motion::Relative => None,
        };
        let run = Box::pin(async move { play(input, &self.playback, &motion, signals).await });
        (run, positions)
    }
}

/// Every key `recording` presses, each once.
fn pressed_keys(recording: &Recording) -> Arc<[Key]> {
    let keys: BTreeSet<Key> = recording
        .events()
        .iter()
        .filter_map(|event| match event.change {
            Change::Press(Held::Key(key)) => Some(key),
            _ => None,
        })
        .collect();
    keys.into_iter().collect()
}

impl<P: Platform> Controller<P> {
    /// The layout changed or became unknown: a playback moving to positions
    /// on another layout stops, and says why.
    pub(super) fn layout_changed(&mut self) {
        let State::Running(running) = &self.state else {
            return;
        };
        let Some(used) = &running.positions else {
            return;
        };
        let why = match self.platform.layout() {
            Some(current) if current == *used => return,
            Some(_) => "the screen layout changed",
            None => "the screen layout is no longer known",
        };
        self.stop(Some(format!("Playback stopped: {why}")));
    }

    /// The playback hotkey while nothing plays: starts the current
    /// recording once the hotkey's modifiers come up. Clicking has to be
    /// stopped first, and while recording no hotkey but its own does
    /// anything.
    pub(super) fn toggle_playback(&mut self) {
        match self.state {
            State::Idle => self.play_from_hotkey(),
            State::RecordCountdown(_) | State::Recording(_) => {}
            State::Countdown(..) | State::Release(..) | State::Opening(_) | State::Running(_) => {
                self.notice = Some("Not playing: clicking is on; stop it first".to_owned());
            }
        }
    }

    /// The tray's Play: loads `file`, counts down, then plays it.
    pub(super) fn play_from_tray(&mut self, file: &FileName) {
        if !matches!(self.state, State::Idle) || !self.may_play() {
            return;
        }
        if let Some(job) = self.load(file) {
            self.start_after_the_tray_delay(job);
        }
    }

    fn play_from_hotkey(&mut self) {
        if !self.may_play() {
            return;
        }
        let Some(job) = self.load_current() else {
            return;
        };
        if let Some(deadline) = Instant::now().checked_add(PLAY_RELEASE_WAIT) {
            self.state = State::Release(deadline, job);
        }
    }

    /// Whether playback may start; says why not when it may not.
    fn may_play(&mut self) -> bool {
        self.session_allows("Not playing")
    }

    /// Loads the current recording: the last one recorded or played, or the
    /// newest when there is none or its file is gone.
    fn load_current(&mut self) -> Option<Job> {
        if let Some(file) = self.current.clone() {
            match self.read(&file) {
                Err(LoadError::Missing) => {}
                read => return self.checked(&file, read),
            }
        }
        self.list_recordings();
        let Some(newest) = self.recordings.first().map(|listed| listed.file.clone()) else {
            let why = self.recordings_error.as_deref();
            self.notice = Some(why.map_or_else(
                || "Nothing to play: no recordings yet".to_owned(),
                |why| format!("Not playing: the recordings cannot be listed: {why}"),
            ));
            return None;
        };
        self.load(&newest)
    }

    /// Reads and checks `file`: what to play, or `None` with a notice
    /// saying why it cannot be played.
    fn load(&mut self, file: &FileName) -> Option<Job> {
        let read = self.read(file);
        self.checked(file, read)
    }

    fn read(&self, file: &FileName) -> Result<Recording, LoadError> {
        let dir = self.platform.recordings_dir().ok_or(LoadError::Missing)?;
        load(&dir, file)
    }

    /// What to play from a file that was read, which becomes the current
    /// recording; or `None` with a notice, and a fresh list, since the one
    /// shown offered a file that cannot be played.
    fn checked(&mut self, file: &FileName, read: Result<Recording, LoadError>) -> Option<Job> {
        let label = file.label();
        let playback = read.map_err(|err| err.to_string()).and_then(|recording| {
            Playback::new(recording, self.config.playback).map_err(|_| {
                format!(
                    "it is shorter than {} ms, too short to play more than once",
                    MIN_LOOP_LENGTH.as_millis()
                )
            })
        });
        // Where the desktop delivers the hotkeys, a recording that presses
        // one of their keys would set it off by itself.
        let playing = playback.and_then(|playback| {
            let keys = pressed_keys(playback.recording());
            match self.own_key_refusal(&OwnKeys::Recording(Arc::clone(&keys))) {
                Some(why) => Err(format!("it presses a hotkey's key: {why}")),
                None => Ok(Playing {
                    playback,
                    label: label.clone(),
                    keys,
                    modifier_held: false,
                }),
            }
        });
        match playing {
            Ok(playing) => {
                self.current = Some(file.clone());
                Some(Job::Play(playing))
            }
            Err(why) => {
                self.notice = Some(format!("Not playing {label}: {why}"));
                self.list_recordings();
                None
            }
        }
    }

    /// A recording was saved at `path`: it is the current one now, and the
    /// list has it.
    pub(super) fn saved(&mut self, path: &Path) {
        let name = path.file_name().and_then(|name| name.to_str());
        if let Some(file) = name.and_then(FileName::parse) {
            self.current = Some(file);
        }
        self.list_recordings();
    }

    /// Lists the recordings again when the tray menu opens, so it offers
    /// what is there now. Only while idle: nothing can be played otherwise,
    /// and a run never waits on a slow folder.
    pub(super) fn list_recordings_while_idle(&mut self) {
        if matches!(self.state, State::Idle) {
            self.list_recordings();
        }
    }

    /// Reads which recordings there are. A folder that cannot be read lists
    /// none and shows as a problem.
    pub(super) fn list_recordings(&mut self) {
        let files = match self.platform.recordings_dir() {
            Some(dir) => list(&dir, LISTED_RECORDINGS),
            None => Ok(Vec::new()),
        };
        self.recordings_error = files.as_ref().err().map(ToString::to_string);
        let listed: Vec<Listed> = files
            .unwrap_or_default()
            .into_iter()
            .map(|file| Listed {
                label: file.label(),
                file,
            })
            .collect();
        if *self.recordings != *listed {
            self.recordings = Arc::from(listed);
        }
    }
}
