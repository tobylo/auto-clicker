//! What the window knows and decides, without its widgets: the settings
//! the daemon last sent, the hotkeys and key captured since, the request it
//! waits on, and what each answer means for what it shows.

use auto_clicker_core::config::{Draft, Field, HotkeysDraft, Reason, Stale};
use auto_clicker_core::controller::{Activity, Route};
use auto_clicker_core::hotkeys::{Action, Capture};
use auto_clicker_core::message::{ChangeFailure, Failure, Quoted, Unavailability};
use auto_clicker_core::recording::{FileName, Rect};
use auto_clicker_core::window::{
    Ask, CaptureEnd, Confirm, Deletion, Files, Located, Outcome, RecordingAction, RecordingOutcome,
    Removal, Request, Since, Spot, WindowView,
};

use crate::model::{Fields, Picked, Unreadable, with_hotkey};

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
    SavedUnsynced(Quoted),
    Busy,
    Failed(Failure),
    Refused(Field, Reason),
    Unavailable(Unavailability),
    Capturing(Target),
    /// A hotkey was captured, as it is written; it is kept on a save.
    CapturedHotkey(String),
    /// The clicker's key was captured: its code.
    CapturedKey(u16),
    CaptureEnded(CaptureEnd),
    Configured,
    /// The overlay to pick a point is up.
    Picking,
    /// A point was picked: where the clicker clicks for it, kept on a save.
    Picked {
        x: u16,
        y: u16,
    },
    PickCancelled,
    PickTimedOut,
    /// The overlay could not be opened: why.
    PickFailed(String),
    Unreadable(Unreadable),
    /// What came of something done with a recording.
    Recording(RecordingOutcome),
    /// What a delete did.
    Deleted(Removal),
    /// A delete deleted nothing: why.
    NotDeleted(ChangeFailure),
}

impl Note {
    /// Whether it says something went wrong.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Note::Failed(_)
                | Note::Refused(..)
                | Note::Unavailable(_)
                | Note::PickFailed(_)
                | Note::Unreadable(_)
                | Note::NotDeleted(_)
                | Note::Recording(
                    RecordingOutcome::NotRenamed(_) | RecordingOutcome::NotPlaying(_)
                )
        ) || matches!(self, Note::Deleted(removal) if removal.fell_short())
    }
}

/// A question a save or a delete asks before it goes ahead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Replace(Stale),
    WithoutModifier(Vec<Action>),
    /// Delete this recording, for good? With what the list calls it.
    DeleteOne(FileName, String),
    /// Delete these recordings, for good?
    DeleteSeveral(Files),
    /// Delete every recording, this many, for good?
    DeleteAll(u64),
}

impl Question {
    /// Whether it asks about a delete other than of `selected` or, for
    /// every recording, of `total`: the selection or the list changed
    /// since it was asked.
    fn outdated(&self, selected: &[FileName], total: u64) -> bool {
        match self {
            Question::DeleteOne(file, _) => selected != std::slice::from_ref(file),
            Question::DeleteSeveral(files) => selected != files.as_slice(),
            Question::DeleteAll(asked) => *asked != total,
            Question::Replace(_) | Question::WithoutModifier(_) => false,
        }
    }

    /// What a yes to it deletes, when it asks about a delete.
    fn deletion(&self) -> Option<Deletion> {
        match self {
            Question::DeleteOne(file, _) => Files::new(vec![file.clone()]).map(Deletion::Files),
            Question::DeleteSeveral(files) => Some(Deletion::Files(files.clone())),
            Question::DeleteAll(_) => Some(Deletion::All),
            Question::Replace(_) | Question::WithoutModifier(_) => None,
        }
    }
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
    Locate,
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
    /// The fixed point picked on the screens: as sent, or as picked since.
    picked: Option<Picked>,
    next_id: u64,
    asked: Option<(u64, Asked)>,
    question: Option<(Question, Agreed)>,
    /// The overlays to pick a point are up: nothing else is asked until
    /// they close.
    picking: bool,
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
            self.picked = Picked::of(&view.settings);
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
            Outcome::Deleted(removal) => Note::Deleted(removal),
            Outcome::NotDeleted(why) => Note::NotDeleted(why),
            Outcome::Located(located) => self.located(located),
        })
    }

    /// Keeps a point the daemon located, for the fields and the next save.
    fn located(&mut self, located: Located) -> Note {
        let Located { x, y, screens } = located;
        self.picked = Some(Picked { x, y, screens });
        Note::Picked { x, y }
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
        let free = self.asked.is_none() && self.question.is_none() && !self.picking;
        self.hotkeys.is_some() && free
    }

    /// The overlays to pick a point are up, or `false`, closed: while they
    /// are up, nothing else is asked.
    pub fn set_picking(&mut self, picking: bool) {
        self.picking = picking;
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
        if let Some(deletion) = question.deletion() {
            self.question = None;
            return Some(Ok(self.send(Ask::Delete(deletion), Asked::Recording)));
        }
        let mut agreed = agreed.clone();
        match question {
            Question::Replace(_) => agreed.replace = true,
            Question::WithoutModifier(actions) => agreed.without_modifier.extend(actions),
            Question::DeleteOne(..) | Question::DeleteSeveral(_) | Question::DeleteAll(_) => {}
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

    /// The recordings `selected` in the list now, of `total`: a delete
    /// asked about others, or about all when there were more or fewer, is
    /// called off, so a yes never deletes what the user did not see asked.
    pub fn selected(&mut self, selected: &[FileName], total: u64) {
        if matches!(&self.question, Some((question, _)) if question.outdated(selected, total)) {
            self.question = None;
        }
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

    /// Does `action` with the recording `file`; `None` while the user may
    /// not ask anything.
    pub fn recording(&mut self, file: FileName, action: RecordingAction) -> Option<Request> {
        if !self.may_ask() {
            return None;
        }
        Some(self.send(Ask::Recording { file, action }, Asked::Recording))
    }

    /// Asks whether to delete the recordings `selected`, the first called
    /// `label` in the list: by that name when it is the only one, else by
    /// how many. Nothing while the user may not ask anything, or with none
    /// selected.
    pub fn delete(&mut self, selected: Vec<FileName>, label: String) {
        if !self.may_ask() {
            return;
        }
        let question = match &selected[..] {
            [one] => Question::DeleteOne(one.clone(), label),
            _ => match Files::new(selected) {
                Some(files) => Question::DeleteSeveral(files),
                None => return,
            },
        };
        self.question = Some((question, Agreed::default()));
    }

    /// Asks whether to delete every recording, `total` of them; nothing
    /// while the user may not ask anything, or with none.
    pub fn delete_all(&mut self, total: u64) {
        if self.may_ask() && total > 0 {
            self.question = Some((Question::DeleteAll(total), Agreed::default()));
        }
    }

    /// Where the clicker clicks for `at`, picked on the screen at `on` in a
    /// pick begun on the counts `since`; `None` while the user may not ask
    /// anything.
    pub fn locate(&mut self, at: Spot, on: Rect, since: Since) -> Option<Request> {
        if !self.may_ask() {
            return None;
        }
        Some(self.send(Ask::Locate { at, on, since }, Asked::Locate))
    }

    fn send_save(
        &mut self,
        fields: &Fields,
        agreed: Agreed,
    ) -> Option<Result<Request, Unreadable>> {
        let draft = fields.to_draft(self.hotkeys.as_ref()?, self.picked.as_ref());
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
pub(crate) mod tests;
