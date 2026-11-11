//! The recordings as the controller keeps them: which there are, which one
//! the playback hotkey plays, and what the settings window does with them.
//!
//! The controller is the only one that lists, renames or deletes a
//! recording, or changes which is current; the tray and the window ask. A
//! listing reads the start of each listed file for its name, and a rename
//! reads and writes a whole file, so both run on the controller's thread
//! only while it is idle, like a load: nothing is pressed then, and no
//! deadline waits.
//!
//! The current recording is kept in a state file, so it survives a restart.
//! It is written when the current recording changes, and only then: a
//! replay of the same one writes nothing.

use std::path::Path;
use std::sync::Arc;

use super::{Controller, LISTED_RECORDINGS, Listed, Platform, State};
use crate::recording::{ChangeError, Changed, CurrentFile, FileName, delete, list, rename};
use crate::window::{Outcome, RecordingAction, RecordingOutcome, RecordingsView};

/// How many recordings the settings window lists, newest first. While it is
/// open, the controller lists this many; else the tray's
/// [`LISTED_RECORDINGS`].
pub const WINDOW_RECORDINGS: usize = 200;

/// The recordings, as last listed, and the current one.
#[derive(Default)]
pub(super) struct Shelf {
    /// The newest recordings, newest first.
    pub(super) listed: Arc<[Listed]>,
    /// How many recordings there were when they were listed.
    total: usize,
    /// The last listing found no file for the current recording.
    current_gone: bool,
    /// Why the recordings could not be listed.
    pub(super) error: Option<String>,
    /// The recording the playback hotkey plays: the one last chosen,
    /// recorded or played. `None` until then, which means the newest.
    pub(super) current: Option<FileName>,
    /// Where the current recording is kept.
    file: CurrentFile,
    /// Why the current recording could not be read at start, or kept.
    pub(super) current_error: Option<String>,
    /// The state file holds the current recording: it was read from there,
    /// or written there.
    kept: bool,
    /// A settings window opened and has not been given its longer list yet.
    relist: bool,
}

impl Shelf {
    /// The listed recording the hotkey plays: the current one, or the
    /// newest when there is none or its file is gone. A current recording
    /// older than the list plays too, and then no listed one is marked.
    pub(super) fn shown_current(&self) -> Option<FileName> {
        let listed = |file: &FileName| self.listed.iter().any(|listed| listed.file == *file);
        match &self.current {
            Some(current) if listed(current) => Some(current.clone()),
            Some(_) if !self.current_gone => None,
            _ => self.listed.first().map(|listed| listed.file.clone()),
        }
    }

    /// What the settings window shows of the recordings.
    pub(super) fn window_view(&self) -> RecordingsView {
        let shown = self.listed.len();
        RecordingsView {
            listed: self.listed.to_vec(),
            more: u64::try_from(self.total.saturating_sub(shown)).unwrap_or(u64::MAX),
            current: self.shown_current(),
        }
    }
}

impl<P: Platform> Controller<P> {
    /// The current recording is kept in `file`: reads it now. One that
    /// cannot be read leaves the newest current, and says why as a problem
    /// until a recording is chosen, made or played.
    pub fn keep_current_file(&mut self, file: CurrentFile) {
        match file.read() {
            Ok(current) => {
                self.shelf.current = current;
                self.shelf.kept = true;
            }
            Err(err) => {
                self.shelf.current_error = Some(format!(
                    "The recording the playback hotkey plays could not be read, so it plays \
                     the newest: {err}"
                ));
            }
        }
        self.shelf.file = file;
    }

    /// Makes `file` the current recording, and keeps it for the next start
    /// unless the state file holds it already.
    pub(super) fn make_current(&mut self, file: &FileName) {
        if self.shelf.current.as_ref() == Some(file) && self.shelf.kept {
            return;
        }
        self.shelf.current = Some(file.clone());
        // It is listed, played or just saved.
        self.shelf.current_gone = false;
        let written = self.shelf.file.write(file);
        self.shelf.kept = written.is_ok();
        self.shelf.current_error = written.err().map(|err| {
            format!(
                "The recording the playback hotkey plays could not be kept for the next start: \
                 {err}"
            )
        });
    }

    /// A recording was saved at `path`: it is the current one now, and the
    /// list has it.
    pub(super) fn saved(&mut self, path: &Path) {
        let name = path.file_name().and_then(|name| name.to_str());
        if let Some(file) = name.and_then(FileName::parse) {
            self.make_current(&file);
        }
        self.list_recordings();
    }

    /// A settings window opened: it gets the longer list once the
    /// controller is idle.
    pub(super) fn relist_for_window(&mut self) {
        self.shelf.relist = true;
        self.relist_when_due();
    }

    /// Lists the recordings for a window that opened, once idle.
    pub(super) fn relist_when_due(&mut self) {
        if self.shelf.relist && matches!(self.state, State::Idle) {
            self.list_recordings();
        }
    }

    /// Lists the recordings again when the tray menu opens, so it offers
    /// what is there now. Only while idle: nothing can be played otherwise,
    /// and a run never waits on a slow folder.
    pub(super) fn list_recordings_while_idle(&mut self) {
        if matches!(self.state, State::Idle) {
            self.list_recordings();
        }
    }

    /// Reads which recordings there are: as many as the window lists while
    /// one is open, else the tray's. A folder that cannot be read lists
    /// none and shows as a problem.
    pub(super) fn list_recordings(&mut self) {
        let limit = if self.settings.window_open() {
            WINDOW_RECORDINGS
        } else {
            LISTED_RECORDINGS
        };
        let current = self.shelf.current.as_ref();
        let found = match self.platform.recordings_dir() {
            Some(dir) => list(&dir, limit, current),
            None => Ok(Default::default()),
        };
        self.shelf.relist = false;
        self.shelf.error = found.as_ref().err().map(ToString::to_string);
        let found = found.unwrap_or_default();
        self.shelf.total = found.total;
        self.shelf.current_gone = self.shelf.current.is_some() && !found.holds;
        let listed: Vec<Listed> = found
            .newest
            .into_iter()
            .map(|found| Listed::new(found.file, found.name))
            .collect();
        if *self.shelf.listed != *listed {
            self.shelf.listed = Arc::from(listed);
        }
    }

    /// What the settings window asked to do with the recording `file`.
    /// Only while idle: a rename reads and writes a whole file, and nothing
    /// changes under a run or a recording.
    pub(super) fn recording_request(
        &mut self,
        file: &FileName,
        action: RecordingAction,
    ) -> Outcome {
        if !self.settled() {
            return Outcome::Busy;
        }
        Outcome::Recording(match action {
            RecordingAction::Rename { name } => {
                let renamed = self.change(|dir| rename(dir, file, name));
                done(
                    renamed,
                    RecordingOutcome::Renamed,
                    ["Renamed", "Not renamed"],
                )
            }
            RecordingAction::Delete => {
                let deleted = self.change(|dir| delete(dir, file));
                done(
                    deleted,
                    RecordingOutcome::Deleted,
                    ["Deleted", "Not deleted"],
                )
            }
            RecordingAction::Choose => {
                self.make_current(file);
                RecordingOutcome::Chosen
            }
            RecordingAction::Play => self.play_from_window(file),
        })
    }

    /// Changes the recordings in their folder with `change`, then lists
    /// them again, whatever came of it: a file found gone was gone from the
    /// list too.
    fn change(
        &mut self,
        change: impl FnOnce(&Path) -> Result<Changed, ChangeError>,
    ) -> Result<Changed, ChangeError> {
        let changed = match self.platform.recordings_dir() {
            Some(dir) => change(&dir),
            None => Err(ChangeError::Missing),
        };
        self.list_recordings();
        changed
    }

    /// Plays `file` as the tray's Play does; what the tray would say when
    /// it does not start.
    fn play_from_window(&mut self, file: &FileName) -> RecordingOutcome {
        self.play_from_tray(file);
        if matches!(self.state, State::Idle) {
            let why = self.notice.clone();
            RecordingOutcome::NotDone(why.unwrap_or_else(|| "Not playing".to_owned()))
        } else {
            RecordingOutcome::Playing
        }
    }
}

/// `ok` when `changed` went through and reached the disk; else a sentence
/// that starts with `done` or `not`.
fn done(
    changed: Result<Changed, ChangeError>,
    ok: RecordingOutcome,
    [done, not]: [&str; 2],
) -> RecordingOutcome {
    match changed {
        Ok(Changed::Synced) => ok,
        Ok(Changed::Unsynced(err)) => RecordingOutcome::Unsynced(format!(
            "{done}, but it may not survive a power loss: {err}"
        )),
        Err(err) => RecordingOutcome::NotDone(format!("{not}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn a_change_says_whether_it_was_done_and_whether_it_reached_the_disk() {
        let said = |changed| {
            done(
                changed,
                RecordingOutcome::Deleted,
                ["Deleted", "Not deleted"],
            )
        };
        assert_eq!(said(Ok(Changed::Synced)), RecordingOutcome::Deleted);
        let unsynced = Changed::Unsynced(io::Error::other("no flush"));
        assert_eq!(
            said(Ok(unsynced)),
            RecordingOutcome::Unsynced(
                "Deleted, but it may not survive a power loss: no flush".to_owned()
            )
        );
        assert_eq!(
            said(Err(ChangeError::Missing)),
            RecordingOutcome::NotDone("Not deleted: the file is no longer there".to_owned())
        );
    }
}
