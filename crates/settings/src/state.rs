//! What the window knows and decides, without its widgets: the settings
//! the daemon last sent, the hotkeys and key captured since, the request it
//! waits on, and what each answer means for what it shows.

use auto_clicker_core::config::{Draft, Field, RouteName, Stale};
use auto_clicker_core::controller::Activity;
use auto_clicker_core::hotkeys::{Action, Capture};
use auto_clicker_core::window::{Ask, CaptureEnd, Confirm, Outcome, Request, WindowView};

use crate::model::{Fields, NotANumber, with_hotkey};

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
    CaptureEnded(CaptureEnd),
    Configured,
    NotANumber(NotANumber),
}

impl Note {
    /// Whether it says something went wrong.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Note::Failed(_) | Note::Refused(..) | Note::Unavailable(_) | Note::NotANumber(_)
        )
    }
}

/// A question a save asks before it goes ahead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Replace(Stale),
    WithoutModifier(Vec<Action>),
}

/// A save, and what the user agreed to for it.
#[derive(Clone, Debug, PartialEq)]
struct Save {
    draft: Draft,
    replace: bool,
    without_modifier: Vec<Action>,
}

/// What the window waits on.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    Save(Save),
    Capture(Target),
    Configure,
}

/// What a new view changes on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// New settings to fill the fields with: the saved ones changed.
    pub fields: Option<Fields>,
    /// A line to show, when an answer came.
    pub note: Option<Note>,
}

/// The window's state.
#[derive(Default)]
pub struct State {
    /// The settings the daemon last sent.
    saved: Option<Draft>,
    /// The hotkeys and the clicker's key to save, captured since.
    keys: Option<Draft>,
    next_id: u64,
    asked: Option<(u64, Asked)>,
    question: Option<(Question, Save)>,
}

impl State {
    /// Takes a view from the daemon.
    pub fn view(&mut self, view: &WindowView) -> Shown {
        let mut shown = Shown::default();
        if self.saved.as_ref() != Some(&view.settings) {
            self.saved = Some(view.settings.clone());
            self.keys = Some(view.settings.clone());
            shown.fields = Some(Fields::from_draft(&view.settings));
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
                self.captured(asked, |keys, target| match target {
                    Target::Hotkey(action) => {
                        keys.hotkeys = with_hotkey(&keys.hotkeys, action, binding.to_string());
                    }
                    Target::Key => keys.clicker.key = binding.key().code(),
                });
                return None;
            }
            Outcome::CapturedKey(code) => {
                self.captured(asked, |keys, _| keys.clicker.key = code);
                return None;
            }
            Outcome::CaptureEnded(why) => Note::CaptureEnded(why),
            Outcome::Configured => Note::Configured,
        })
    }

    /// Keeps a question a save asked, to ask the user.
    fn ask(&mut self, confirm: Confirm, asked: Asked) -> Option<Note> {
        let Asked::Save(save) = asked else {
            return None;
        };
        let question = match confirm {
            Confirm::Replace(stale) => Question::Replace(stale),
            Confirm::WithoutModifier(actions) => Question::WithoutModifier(actions),
        };
        self.question = Some((question, save));
        None
    }

    fn captured(&mut self, asked: Asked, set: impl FnOnce(&mut Draft, Target)) {
        if let (Asked::Capture(target), Some(keys)) = (asked, &mut self.keys) {
            set(keys, target);
        }
    }

    /// The question to show, if a save asked one.
    pub fn question(&self) -> Option<&Question> {
        self.question.as_ref().map(|(question, _)| question)
    }

    /// Whether the window waits on an answer.
    pub fn waiting(&self) -> bool {
        self.asked.is_some()
    }

    /// The hotkeys and key to show: as captured, before a save.
    pub fn keys(&self) -> Option<&Draft> {
        self.keys.as_ref()
    }

    /// A save of `fields`; what is wrong instead, when a number is not one.
    /// `None` before the daemon sent the settings: there is nothing to save
    /// yet.
    pub fn save(&mut self, fields: &Fields) -> Option<Result<Request, NotANumber>> {
        let draft = fields.to_draft(self.keys.as_ref()?);
        Some(draft.map(|draft| {
            self.send_save(Save {
                draft,
                replace: false,
                without_modifier: Vec::new(),
            })
        }))
    }

    /// The user said yes to the question: the save again, agreeing to it.
    pub fn agree(&mut self) -> Option<Request> {
        let (question, mut save) = self.question.take()?;
        match question {
            Question::Replace(_) => save.replace = true,
            Question::WithoutModifier(actions) => save.without_modifier.extend(actions),
        }
        Some(self.send_save(save))
    }

    /// The user said no: nothing is saved.
    pub fn decline(&mut self) {
        self.question = None;
    }

    /// A capture for `target`.
    pub fn capture(&mut self, target: Target) -> Request {
        let capture = match target {
            Target::Hotkey(_) => Capture::Hotkey,
            Target::Key => Capture::Key,
        };
        self.send(Ask::Capture(capture), Asked::Capture(target))
    }

    /// A request for the desktop's shortcut settings.
    pub fn configure(&mut self) -> Request {
        let ask = Ask::ConfigureShortcuts {
            activation_token: None,
        };
        self.send(ask, Asked::Configure)
    }

    fn send_save(&mut self, save: Save) -> Request {
        let ask = Ask::Save {
            settings: save.draft.clone(),
            replace: save.replace,
            without_modifier: save.without_modifier.clone(),
        };
        self.send(ask, Asked::Save(save))
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

/// Whether the desktop delivers the hotkeys now.
pub fn desktop_hotkeys(view: &WindowView) -> bool {
    view.running.hotkeys == RouteName::Portal
}

/// Whether the saved routes differ from those the daemon runs on: they
/// apply from its next start.
pub fn routes_pending(view: &WindowView) -> bool {
    view.settings.input != view.running
}

#[cfg(test)]
mod tests;
