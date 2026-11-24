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
use crate::log;
use crate::message::{ChangeFailure, Problem, Quoted, Titled};
use crate::recording::{
    ChangeError, Changed, CurrentFile, Doomed, FileName, Removed, delete, list, remove_stale,
    rename,
};
use crate::window::{
    Deletion, MAX_FAILURES, NotDeleted, Outcome, RecordingAction, RecordingOutcome, RecordingsView,
    Removal,
};

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
    pub(super) error: Option<Quoted>,
    /// The recording the playback hotkey plays: the one last chosen,
    /// recorded or played. `None` until then, which means the newest.
    pub(super) current: Option<FileName>,
    /// Where the current recording is kept.
    file: CurrentFile,
    /// Why the current recording could not be read at start, or kept.
    pub(super) current_error: Option<Problem>,
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
    /// Removes the temporary files saves left when they crashed: beside the
    /// recordings, the config and the current-recording file. Only those an
    /// hour old or more go, so a save still running in another session
    /// keeps its file. One that cannot be removed is logged and left.
    pub(super) fn remove_stale_temps(&self) {
        let recordings = self.platform.recordings_dir().map(|dir| remove_stale(&dir));
        let found = [
            ("recordings", recordings.unwrap_or(Ok(0))),
            ("config", self.settings.file.remove_stale_temps()),
            ("state", self.shelf.file.remove_stale_temps()),
        ];
        for (folder, result) in found {
            if let Err(err) = result {
                log::line(format_args!(
                    "auto-clicker: could not clear old temporary files in the {folder} folder: {err}"
                ));
            }
        }
    }

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
                let why = Quoted::new(err.to_string());
                self.shelf.current_error = Some(Problem::CurrentUnread(why));
            }
        }
        self.shelf.file = file;
    }

    /// Makes `file` the current recording, and keeps it for the next start
    /// unless the state file holds it already.
    pub(super) fn make_current(&mut self, file: &FileName) {
        // It was just played or saved; a choice lists again after this.
        self.shelf.current_gone = false;
        if self.shelf.current.as_ref() == Some(file) && self.shelf.kept {
            return;
        }
        self.shelf.current = Some(file.clone());
        let written = self.shelf.file.write(file);
        self.shelf.kept = written.is_ok();
        self.shelf.current_error = written
            .err()
            .map(|err| Problem::CurrentUnkept(Quoted::new(err.to_string())));
    }

    /// A recording was saved at `path`: it is the current one now, and the
    /// list has it.
    /// The file, when its name is one a recording's file has.
    pub(super) fn saved(&mut self, path: &Path) -> Option<FileName> {
        let name = path.file_name().and_then(|name| name.to_str());
        let file = name.and_then(FileName::parse);
        if let Some(file) = &file {
            self.make_current(file);
        }
        self.list_recordings();
        file
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
        self.shelf.error = found.as_ref().err().map(|err| Quoted::new(err.to_string()));
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
                renamed_outcome(renamed)
            }
            RecordingAction::Choose => {
                self.make_current(file);
                // The list shows whether its file is still there.
                self.list_recordings();
                RecordingOutcome::Chosen
            }
            RecordingAction::Play => self.play_from_window(file),
        })
    }

    /// What the settings window asked to delete. Only while idle, like
    /// the other changes: nothing that plays or counts down is deleted, and
    /// no stop waits on the files. A recording the playback hotkey plays
    /// that goes leaves the newest to play, as the list shows.
    pub(super) fn delete_request(&mut self, which: &Deletion) -> Outcome {
        if !self.settled() {
            return Outcome::Busy;
        }
        let doomed = match which {
            Deletion::Files(files) => Doomed::These(files.as_slice()),
            Deletion::All => Doomed::All,
        };
        // The names the list showed, for the files that stay.
        let listed = Arc::clone(&self.shelf.listed);
        match self.change(|dir| delete(dir, doomed, MAX_FAILURES)) {
            Ok(removed) => Outcome::Deleted(removal(removed, &listed)),
            Err(err) => Outcome::NotDeleted(ChangeFailure::from(&err)),
        }
    }

    /// Changes the recordings in their folder with `change`, then lists
    /// them again, whatever came of it: a file found gone was gone from the
    /// list too.
    fn change<T>(
        &mut self,
        change: impl FnOnce(&Path) -> Result<T, ChangeError>,
    ) -> Result<T, ChangeError> {
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
            RecordingOutcome::NotPlaying(self.notice.clone())
        } else {
            RecordingOutcome::Playing
        }
    }
}

/// Renamed when the rename went through and reached the disk; else how it
/// went short of that.
fn renamed_outcome(changed: Result<Changed, ChangeError>) -> RecordingOutcome {
    match changed {
        Ok(Changed::Synced) => RecordingOutcome::Renamed,
        Ok(Changed::Unsynced(err)) => RecordingOutcome::Unsynced(Quoted::new(err.to_string())),
        Err(err) => RecordingOutcome::NotRenamed(ChangeFailure::from(&err)),
    }
}

/// What the window is told of `removed`, naming each file left by its
/// name in `listed`, when it is there.
fn removal(removed: Removed, listed: &[Listed]) -> Removal {
    let titled = |file: &FileName| {
        let name = listed
            .iter()
            .find(|listed| listed.file == *file)
            .and_then(|listed| listed.name.as_deref());
        Titled::new(file.clone(), name)
    };
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    Removal {
        deleted: count(removed.deleted),
        failed: removed
            .failed
            .iter()
            .map(|(file, err)| NotDeleted {
                recording: titled(file),
                why: ChangeFailure::from(err),
            })
            .collect(),
        more_failed: count(removed.more_failed),
        unsynced: removed.unsynced.map(|err| Quoted::new(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn a_rename_says_whether_it_was_done_and_whether_it_reached_the_disk() {
        let said = |changed| renamed_outcome(changed).to_string();
        assert_eq!(said(Ok(Changed::Synced)), "Renamed.");
        let unsynced = Changed::Unsynced(io::Error::other("no flush"));
        assert_eq!(
            said(Ok(unsynced)),
            "Renamed, but it may not survive a power loss: no flush"
        );
        assert_eq!(
            said(Err(ChangeError::Missing)),
            "Not renamed: the file is no longer there"
        );
    }

    #[test]
    fn a_delete_counts_what_went_and_names_what_did_not() {
        let file = |at: &str| FileName::parse(&format!("2026-11-09_{at}.000.json")).unwrap();
        let listed = [Listed::new(file("18-01-00"), Some("Farm loop".to_owned()))];
        let removed = Removed {
            deleted: 2,
            failed: vec![
                (file("18-00-00"), ChangeError::Missing),
                (file("18-01-00"), ChangeError::Missing),
            ],
            more_failed: 3,
            unsynced: Some(io::Error::other("no flush")),
        };
        assert_eq!(
            removal(removed, &listed).to_string(),
            "Deleted 2 recordings, but this may not survive a power loss: no flush. \
             Not deleted: Recording 2026-11-09 18:00:00: the file is no longer there; \
             Farm loop (2026-11-09 18:01:00): the file is no longer there. \
             3 more were not deleted."
        );
        let one = Removed {
            deleted: 1,
            ..Removed::default()
        };
        assert_eq!(removal(one, &[]).to_string(), "Deleted 1 recording.");
    }

    #[test]
    fn a_delete_that_found_nothing_says_so_and_one_that_only_failed_says_only_that() {
        let nothing = Removal::default();
        assert!(!nothing.fell_short());
        assert_eq!(nothing.to_string(), "Deleted 0 recordings.");
        let uncounted = Removal {
            more_failed: 2,
            ..Removal::default()
        };
        assert!(uncounted.fell_short());
        assert_eq!(uncounted.to_string(), "2 more were not deleted.");
        let one = Removal {
            deleted: 1,
            more_failed: 1,
            ..Removal::default()
        };
        assert_eq!(
            one.to_string(),
            "Deleted 1 recording. 1 more was not deleted."
        );
    }
}
