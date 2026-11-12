//! The tray's notices and the answers about a recording, in the window's
//! language: what the window shows when Play, Rename or Delete does not
//! go as asked.

use auto_clicker_core::hotkeys::Refusal;
use auto_clicker_core::message::{
    ChangeFailure, Hindrance, Notice, PlayStart, RecordingChange, SaveFailure, Saving, Start, Step,
    Stop, Unplayable,
};
use auto_clicker_core::playback::Fallback;
use auto_clicker_core::window::RecordingOutcome;
use slint::SharedString;

use super::{Said, s};
use crate::model::key_text;
use crate::ui::Texts;

impl Said<'_> {
    pub fn recording(&self, outcome: &RecordingOutcome) -> SharedString {
        let t = self.texts;
        match outcome {
            RecordingOutcome::Renamed => t.invoke_renamed(),
            RecordingOutcome::Deleted => t.invoke_deleted(),
            RecordingOutcome::Chosen => t.invoke_chosen(),
            RecordingOutcome::Playing => t.invoke_playing(),
            RecordingOutcome::Unsynced(RecordingChange::Rename, why) => {
                t.invoke_unsynced_renamed(s(why))
            }
            RecordingOutcome::Unsynced(RecordingChange::Delete, why) => {
                t.invoke_unsynced_deleted(s(why))
            }
            RecordingOutcome::NotChanged(RecordingChange::Rename, why) => {
                t.invoke_not_renamed(self.change_failure(why))
            }
            RecordingOutcome::NotChanged(RecordingChange::Delete, why) => {
                t.invoke_not_deleted(self.change_failure(why))
            }
            RecordingOutcome::NotPlaying(Some(notice)) => self.notice(notice),
            RecordingOutcome::NotPlaying(None) => t.invoke_not_playing(),
        }
    }

    fn change_failure(&self, why: &ChangeFailure) -> SharedString {
        let t = self.texts;
        match why {
            ChangeFailure::Missing => t.invoke_file_missing(),
            ChangeFailure::Refused(why) | ChangeFailure::Folder(why) => s(why),
            ChangeFailure::Unreadable(why) => self.unplayable(why),
            ChangeFailure::NotDone {
                step: Step::Written,
                why,
            } => t.invoke_file_not_written(s(why)),
            ChangeFailure::NotDone {
                step: Step::Deleted,
                why,
            } => t.invoke_file_not_deleted(s(why)),
            ChangeFailure::TooLarge { mib } => t.invoke_rename_too_large(s(mib)),
        }
    }

    pub fn notice(&self, notice: &Notice) -> SharedString {
        let t = self.texts;
        match notice {
            Notice::Refused { start, why } => {
                let refused = match start {
                    Start::Clicking => Texts::invoke_refused_clicking,
                    Start::Playback => Texts::invoke_refused_playback,
                    Start::Recording => Texts::invoke_refused_recording,
                };
                refused(t, self.hindrance(why))
            }
            Notice::Unplayable { recording, why } => {
                t.invoke_unplayable(s(recording), self.unplayable(why))
            }
            Notice::NothingToPlay => t.invoke_nothing_to_play(),
            Notice::Stopped(why) => self.stop(why),
            Notice::RunFailed { playback, why } => {
                let failed = if *playback {
                    Texts::invoke_run_failed_playback
                } else {
                    Texts::invoke_run_failed_clicking
                };
                failed(t, self.input(why))
            }
            Notice::Finished(Some(recording)) => t.invoke_finished_playing(s(recording)),
            Notice::Finished(None) => t.invoke_finished_all(),
            Notice::Playing(start) => self.play_start(*start),
            Notice::Recorded {
                stop: Some(why),
                saving,
            } => t.invoke_two_sentences(self.stop(why), self.saving(saving)),
            Notice::Recorded { stop: None, saving } => self.saving(saving),
        }
    }

    fn hindrance(&self, why: &Hindrance) -> SharedString {
        let t = self.texts;
        match why {
            Hindrance::Blocked(why) => self.blocked(why),
            Hindrance::OwnKey(why) => self.own_key(why),
            Hindrance::Input(why) => self.input(why),
            Hindrance::HeldTooLong { seconds } => t.invoke_held_too_long(s(seconds)),
            Hindrance::ReleaseLost(hotkey) => t.invoke_release_lost(s(hotkey)),
            Hindrance::HotkeysStopped => t.invoke_hotkeys_stopped(),
            Hindrance::ClickingOn => t.invoke_clicking_on(),
            Hindrance::Unlisted(why) => t.invoke_unlisted(s(why)),
            Hindrance::NoRecording(why) => self.no_recording(why),
            Hindrance::NoClock => t.invoke_no_clock(),
            Hindrance::Hotkeys(why) => self.unavailable(why),
        }
    }

    fn unplayable(&self, why: &Unplayable) -> SharedString {
        let t = self.texts;
        match why {
            Unplayable::Missing => t.invoke_file_missing(),
            Unplayable::Unreadable(why) => t.invoke_file_unreadable(s(why)),
            Unplayable::TooLarge { mib } => t.invoke_file_too_large(s(mib)),
            Unplayable::Invalid(why) => t.invoke_file_invalid(s(why)),
            Unplayable::TooShort { ms } => t.invoke_too_short(s(ms)),
            Unplayable::PressesHotkeyKey(why) => t.invoke_presses_hotkey_key(self.own_key(why)),
        }
    }

    fn stop(&self, why: &Stop) -> SharedString {
        let t = self.texts;
        match why {
            Stop::Blocked(why) => t.invoke_stopped(self.blocked(why)),
            Stop::OwnKey(why) => t.invoke_stopped(self.own_key(why)),
            Stop::LayoutChanged => t.invoke_layout_changed(),
            Stop::LayoutUnknown => t.invoke_layout_unknown(),
            Stop::Hotkeys(why) => t.invoke_recording_stopped(self.unavailable(why)),
            Stop::Full => t.invoke_recording_full(),
            Stop::Limit { minutes } => t.invoke_recording_limit(s(minutes)),
        }
    }

    fn play_start(&self, start: PlayStart) -> SharedString {
        let t = self.texts;
        match start {
            PlayStart::Held { seconds } => t.invoke_played_held(s(seconds)),
            PlayStart::Relative(why) => self.fallback(why),
            PlayStart::HeldAndRelative { seconds, fallback } => {
                t.invoke_two_sentences(t.invoke_played_held(s(seconds)), self.fallback(fallback))
            }
        }
    }

    fn fallback(&self, why: Fallback) -> SharedString {
        match why {
            Fallback::Unknown => self.texts.invoke_fallback_unknown(),
            Fallback::Changed => self.texts.invoke_fallback_changed(),
        }
    }

    fn saving(&self, saving: &Saving) -> SharedString {
        let t = self.texts;
        match saving {
            Saving::Nothing => t.invoke_saved_nothing(),
            Saving::Saved(name) => t.invoke_saved_as(s(name)),
            Saving::Failed(SaveFailure::NoDataDirectory) => {
                t.invoke_save_failed(t.invoke_no_data_directory())
            }
            Saving::Failed(SaveFailure::Invalid(why)) => {
                t.invoke_save_failed(t.invoke_recorded_invalid(s(why)))
            }
            Saving::Failed(SaveFailure::Write(why)) => t.invoke_save_failed(s(why)),
        }
    }

    fn own_key(&self, why: &Refusal) -> SharedString {
        let t = self.texts;
        let key = |key: auto_clicker_core::input::Key| s(key_text(key.code()));
        match why {
            Refusal::Modifier(k) => t.invoke_own_key_modifier(key(*k)),
            Refusal::HotkeyKey { key: k, trigger } => t.invoke_own_key_hotkey(key(*k), s(trigger)),
            Refusal::Unreadable(trigger) => t.invoke_own_key_unreadable(s(trigger)),
            Refusal::NotReported => t.invoke_own_key_not_reported(),
        }
    }
}
