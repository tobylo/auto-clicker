//! What the window knows and decides, without its widgets: the settings
//! the daemon last sent, the hotkeys and key captured since, the request it
//! waits on, and what each answer means for what it shows.

use auto_clicker_core::config::{Draft, Field, HotkeysDraft, Stale};
use auto_clicker_core::controller::{Activity, Route};
use auto_clicker_core::hotkeys::{Action, Capture};
use auto_clicker_core::recording::FileName;
use auto_clicker_core::window::{
    Ask, CaptureEnd, Confirm, Outcome, RecordingAction, RecordingOutcome, Request, WindowView,
};

use crate::model::{Fields, Unreadable, with_hotkey};

/// What a capture is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The hotkey of the action at this place (0 to 2).
    Hotkey(i32),
    Key,
}

/// A line the window shows under its tabs.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    Saved,
    SavedUnsynced(String),
    Busy,
    Failed(String),
    Refused(Field, String),
    Unavailable(String),
    Capturing(Target),
    /// A hotkey was captured, as it is written; it is kept on a save.
    CapturedHotkey(String),
    /// The clicker's key was captured: its code.
    CapturedKey(u16),
    CaptureEnded(CaptureEnd),
    Configured,
    Unreadable(Unreadable),
    /// What came of something done with a recording.
    Recording(RecordingOutcome),
}

impl Note {
    /// Whether it says something went wrong.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Note::Failed(_)
                | Note::Refused(..)
                | Note::Unavailable(_)
                | Note::Unreadable(_)
                | Note::Recording(RecordingOutcome::NotDone(_))
        )
    }
}

/// A question a save or a delete asks before it goes ahead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Replace(Stale),
    WithoutModifier(Vec<Action>),
    /// Delete this recording, for good? With what the list calls it.
    Delete(FileName, String),
}

/// What the user agreed to for a save.
#[derive(Clone, Debug, Default, PartialEq)]
struct Agreed {
    replace: bool,
    without_modifier: Vec<Action>,
}

/// What the window waits on.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    Save(Agreed),
    Capture(Target),
    Configure,
    Recording,
}

/// What a new view changes on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// New settings to fill the fields with: the saved ones changed.
    pub fields: Option<Draft>,
    /// A line to show, when an answer came.
    pub note: Option<Note>,
}

/// The window's state.
#[derive(Default)]
pub struct State {
    /// The settings the daemon last sent.
    saved: Option<Draft>,
    /// The hotkeys to save: as sent, or as captured since.
    hotkeys: Option<HotkeysDraft>,
    next_id: u64,
    asked: Option<(u64, Asked)>,
    question: Option<(Question, Agreed)>,
}

impl State {
    /// Takes a view from the daemon.
    pub fn view(&mut self, view: &WindowView) -> Shown {
        let mut shown = Shown::default();
        // The fields are not edited while a request waits on its answer,
        // so settings saved by it can fill them, as the daemon has them.
        if self.saved.as_ref() != Some(&view.settings) {
            self.saved = Some(view.settings.clone());
            self.hotkeys = Some(view.settings.hotkeys.clone());
            shown.fields = Some(view.settings.clone());
        }
        let answered = view.answer.as_ref().and_then(|answer| {
            let (id, _) = self.asked.as_ref()?;
            (answer.id == *id).then_some(&answer.outcome)
        });
        if let Some(outcome) = answered.cloned() {
            shown.note = self.answered(outcome);
        }
        shown
    }

    /// What an answer to the request asked means; the request is over.
    fn answered(&mut self, outcome: Outcome) -> Option<Note> {
        let (_, asked) = self.asked.take()?;
        Some(match outcome {
            Outcome::Saved => Note::Saved,
            Outcome::SavedUnsynced(why) => Note::SavedUnsynced(why),
            Outcome::Busy => Note::Busy,
            Outcome::Failed(why) => Note::Failed(why),
            Outcome::Refused(refusal) => Note::Refused(refusal.field, refusal.reason),
            Outcome::Unavailable(why) => Note::Unavailable(why),
            Outcome::Confirm(confirm) => return self.ask(confirm, asked),
            Outcome::CapturedHotkey(binding) => {
                let text = binding.to_string();
                if let (Asked::Capture(Target::Hotkey(action)), Some(hotkeys)) =
                    (asked, &mut self.hotkeys)
                {
                    *hotkeys = with_hotkey(hotkeys, action, text.clone());
                }
                Note::CapturedHotkey(text)
            }
            Outcome::CapturedKey(code) => Note::CapturedKey(code),
            Outcome::CaptureEnded(why) => Note::CaptureEnded(why),
            Outcome::Configured => Note::Configured,
            Outcome::Recording(outcome) => Note::Recording(outcome),
        })
    }

    /// Keeps a question a save asked, to ask the user.
    fn ask(&mut self, confirm: Confirm, asked: Asked) -> Option<Note> {
        let Asked::Save(agreed) = asked else {
            return None;
        };
        let question = match confirm {
            Confirm::Replace(stale) => Question::Replace(stale),
            Confirm::WithoutModifier(actions) => Question::WithoutModifier(actions),
        };
        self.question = Some((question, agreed));
        None
    }

    /// The question to show, if a save asked one.
    pub fn question(&self) -> Option<&Question> {
        self.question.as_ref().map(|(question, _)| question)
    }

    /// Whether the window waits on an answer.
    pub fn waiting(&self) -> bool {
        self.asked.is_some()
    }

    /// The hotkeys to show: as captured, before a save.
    pub fn hotkeys(&self) -> Option<&HotkeysDraft> {
        self.hotkeys.as_ref()
    }

    /// Whether the user may ask something now: the settings came, nothing
    /// waits on an answer and no question is open.
    pub fn may_ask(&self) -> bool {
        self.hotkeys.is_some() && self.asked.is_none() && self.question.is_none()
    }

    /// A save of `fields`; which field cannot be read instead, if one
    /// cannot. `None` while the user may not ask anything.
    pub fn save(&mut self, fields: &Fields) -> Option<Result<Request, Unreadable>> {
        if !self.may_ask() {
            return None;
        }
        self.send_save(fields, Agreed::default())
    }

    /// The user said yes to the question: a save of the fields as they are
    /// now, agreeing to it. `None` with no question open, or while an
    /// answer is still awaited.
    /// Fields that cannot be read keep the question open, for the user to
    /// correct them and say yes again.
    pub fn agree(&mut self, fields: &Fields) -> Option<Result<Request, Unreadable>> {
        if self.asked.is_some() {
            return None;
        }
        let (question, agreed) = self.question.as_ref()?;
        let mut agreed = agreed.clone();
        match question {
            Question::Replace(_) => agreed.replace = true,
            Question::WithoutModifier(actions) => agreed.without_modifier.extend(actions),
            Question::Delete(file, _) => {
                let file = file.clone();
                self.question = None;
                return Some(Ok(self.send_recording(file, RecordingAction::Delete)));
            }
        }
        let sent = self.send_save(fields, agreed);
        if matches!(sent, Some(Ok(_))) {
            self.question = None;
        }
        sent
    }

    /// The user said no: nothing is saved.
    pub fn decline(&mut self) {
        self.question = None;
    }

    /// A capture for `target`; `None` while the user may not ask anything.
    pub fn capture(&mut self, target: Target) -> Option<Request> {
        if !self.may_ask() {
            return None;
        }
        let capture = match target {
            Target::Hotkey(_) => Capture::Hotkey,
            Target::Key => Capture::Key,
        };
        Some(self.send(Ask::Capture(capture), Asked::Capture(target)))
    }

    /// A request for the desktop's shortcut settings; `None` while the user
    /// may not ask anything.
    pub fn configure(&mut self) -> Option<Request> {
        if !self.may_ask() {
            return None;
        }
        let ask = Ask::ConfigureShortcuts {
            activation_token: None,
        };
        Some(self.send(ask, Asked::Configure))
    }

    /// Does `action` with the recording `file`; a delete asks first.
    /// `None` while the user may not ask anything, or when it asks.
    pub fn recording(
        &mut self,
        file: FileName,
        label: String,
        action: RecordingAction,
    ) -> Option<Request> {
        if !self.may_ask() {
            return None;
        }
        if action == RecordingAction::Delete {
            self.question = Some((Question::Delete(file, label), Agreed::default()));
            return None;
        }
        Some(self.send_recording(file, action))
    }

    fn send_recording(&mut self, file: FileName, action: RecordingAction) -> Request {
        self.send(Ask::Recording { file, action }, Asked::Recording)
    }

    fn send_save(
        &mut self,
        fields: &Fields,
        agreed: Agreed,
    ) -> Option<Result<Request, Unreadable>> {
        let draft = fields.to_draft(self.hotkeys.as_ref()?);
        Some(draft.map(|settings| {
            let ask = Ask::Save {
                settings,
                replace: agreed.replace,
                without_modifier: agreed.without_modifier.clone(),
            };
            self.send(ask, Asked::Save(agreed))
        }))
    }

    fn send(&mut self, ask: Ask, asked: Asked) -> Request {
        self.next_id = self.next_id.wrapping_add(1);
        self.asked = Some((self.next_id, asked));
        Request {
            id: self.next_id,
            ask,
        }
    }
}

/// Whether the daemon is idle, so a save or a capture may go.
pub fn idle(view: &WindowView) -> bool {
    view.activity == Activity::Idle
}

/// Whether the desktop delivers the hotkeys now: the route the daemon
/// picked at its start, whatever the config named (`auto` included).
pub fn desktop_hotkeys(view: &WindowView) -> bool {
    view.routes.hotkeys == Route::Portal
}

/// Whether the saved routes differ from those the daemon runs on: they
/// apply from its next start.
pub fn routes_pending(view: &WindowView) -> bool {
    view.settings.input != view.running
}

#[cfg(test)]
mod tests;
