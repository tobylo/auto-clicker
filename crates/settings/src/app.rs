//! Between the window's widgets and its [`State`]: shows what the daemon
//! sends, and turns the user's choices into requests.

use std::cell::RefCell;
use std::rc::Rc;

use auto_clicker_core::config::Field;
use auto_clicker_core::config::Stale;
use auto_clicker_core::hotkeys::Action;
#[cfg(windows)]
use auto_clicker_core::hotkeys::Capture;
use auto_clicker_core::recording::FileName;
#[cfg(windows)]
use auto_clicker_core::window::Pressed;
use auto_clicker_core::window::{
    CaptureEnd, RecordingAction, RecordingsView, Request, Since, Update, WindowView,
};
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::link::Link;
use crate::model::{Fields, Unreadable, hotkey_text, key_text};
use crate::pick::Pick;
use crate::said::Said;
use crate::shelf::{self, Click, Selection};
use crate::state::{self, Note, Question, State, Target};
use crate::ui::{RecordingRow, SettingsWindow, Texts};

/// The window, its link to the daemon and its state.
pub struct App {
    window: SettingsWindow,
    link: Link,
    state: State,
    /// The daemon's last view.
    last: Option<WindowView>,
    /// The decimal separator of the window's language.
    decimal: char,
    /// The recordings selected in the list.
    selection: Selection,
    /// The recording whose name the name field was filled with: the one
    /// selected alone, when it was.
    named: Option<FileName>,
    /// The list's rows, changed in place when only the selection changes,
    /// so the list keeps where it is scrolled to.
    rows: Rc<VecModel<RecordingRow>>,
    /// The overlays that pick the fixed point, while they are up, and the
    /// counts the pick is checked against.
    pick: Option<(Pick, Since)>,
}

impl App {
    /// Connects the window's callbacks to a new app over `link`.
    pub fn start(window: &SettingsWindow, link: Link, decimal: char) -> Rc<RefCell<App>> {
        // Linux's routes are neither shown nor changed on Windows.
        window.set_on_windows(cfg!(windows));
        let app = Rc::new(RefCell::new(App {
            window: window.clone_strong(),
            link,
            state: State::default(),
            last: None,
            decimal,
            selection: Selection::default(),
            named: None,
            rows: Rc::new(VecModel::default()),
            pick: None,
        }));
        window.set_recordings(ModelRc::from(Rc::clone(&app.borrow().rows)));
        let on = |act: fn(&mut App)| {
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    act(&mut app.borrow_mut());
                }
            }
        };
        window.on_save(on(App::save));
        window.on_capture_key(on(|app| app.capture(Target::Key)));
        window.on_configure_shortcuts(on(App::configure));
        window.on_confirm(on(App::agree));
        window.on_dismiss(on(App::decline));
        window.on_rename_recording(on(App::rename));
        window.on_delete_recordings(on(App::delete));
        window.on_delete_all_recordings(on(App::delete_all));
        window.on_choose_recording(on(|app| app.recording(RecordingAction::Choose)));
        window.on_play_recording(on(|app| app.recording(RecordingAction::Play)));
        let weak = Rc::downgrade(&app);
        window.on_pick_point(move || {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().pick(Rc::downgrade(&app));
            }
        });
        // The overlays close with the window, so the program ends with it.
        let weak = Rc::downgrade(&app);
        window.window().on_close_requested(move || {
            if let Some(app) = weak.upgrade() {
                let _ = app.borrow_mut().close_pick();
            }
            CloseRequestResponse::HideWindow
        });
        App::connect_selection(window, &app);
        let weak = Rc::downgrade(&app);
        window.on_capture_hotkey(move |action| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().capture(Target::Hotkey(action));
            }
        });
        app
    }

    /// Connects the list's clicks and keys to the selection of `app`.
    fn connect_selection(window: &SettingsWindow, app: &Rc<RefCell<App>>) {
        let weak = Rc::downgrade(app);
        window.on_press_recording(move |index, toggle, range| {
            let click = match (toggle, range) {
                (_, true) => Click::Range,
                (true, false) => Click::Toggle,
                (false, false) => Click::Only,
            };
            if let Some(app) = weak.upgrade() {
                app.borrow_mut()
                    .select_with(|selection, view| selection.click(view, index, click));
            }
        });
        let weak = Rc::downgrade(app);
        window.on_step_recordings(move |by, extend| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut()
                    .select_with(|selection, view| selection.step(view, by, extend));
            }
        });
        let weak = Rc::downgrade(app);
        window.on_select_all_recordings(move || {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().select_with(Selection::all);
            }
        });
    }

    /// Takes an update from the daemon.
    pub fn update(&mut self, update: Update) {
        match update {
            Update::View(view) => self.view(&view),
            // Shows the window if it is hidden, and brings it forward where
            // the system lets it. On Wayland a window cannot bring itself
            // forward or out of the minimised state: that needs an
            // activation token from the tray's click, which neither the menu
            // nor Slint's Wayland-only build can pass on yet. On Windows the
            // daemon lets it.
            Update::Raise => {
                let _ = self.window.show();
                crate::system::bring_forward(self.window.window());
            }
        }
    }

    fn view(&mut self, view: &WindowView) {
        self.pick_follows(view);
        let shown = self.state.view(view);
        if let Some(settings) = &shown.fields {
            self.load(&Fields::from_draft(settings, self.decimal));
        }
        let relisted = self.last.as_ref().map(|last| &last.recordings) != Some(&view.recordings);
        self.last = Some(view.clone());
        let w = &self.window;
        w.set_idle(state::idle(view));
        w.set_screens_known(view.screens.is_ok());
        w.set_desktop_hotkeys(state::desktop_hotkeys(view));
        w.set_routes_pending(state::routes_pending(view));
        let texts = w.global::<Texts>();
        let said = self.said(&texts);
        let screens_why = view.screens.as_ref().err();
        w.set_screens_why(
            screens_why
                .map(|why| said.no_screens(*why))
                .unwrap_or_default(),
        );
        w.set_no_recording(
            view.no_recording
                .as_ref()
                .map(|why| said.no_recording(why))
                .unwrap_or_default(),
        );
        w.set_on_screen_why(
            view.no_on_screen
                .map(|why| said.no_on_screen(why))
                .unwrap_or_default(),
        );
        let problems: Vec<SharedString> = view.problems.iter().map(|p| said.problem(p)).collect();
        w.set_problems(problems.join("\n").into());
        if relisted {
            self.show_recordings(view);
        }
        self.show_hotkeys();
        if let Some(note) = shown.note {
            self.note(&note);
        }
        self.show_question();
    }

    /// Fills the fields with saved settings.
    fn load(&self, fields: &Fields) {
        let w = &self.window;
        w.set_tray_delay(fields.tray_delay);
        w.set_mode(fields.mode);
        w.set_interval_text(fields.interval_text.as_str().into());
        w.set_interval_unit(fields.interval_unit);
        w.set_jitter(fields.jitter);
        w.set_button(fields.button);
        w.set_key_text(fields.key_text.as_str().into());
        w.set_position_fixed(fields.position_fixed);
        w.set_position_x(fields.position_x);
        w.set_position_y(fields.position_y);
        w.set_repeat_forever(fields.repeat_forever);
        w.set_repeat_text(fields.repeat_text.as_str().into());
        w.set_output_route(fields.output_route);
        w.set_hotkeys_route(fields.hotkeys_route);
        w.set_speed_text(fields.speed_text.as_str().into());
        w.set_plays_forever(fields.plays_forever);
        w.set_plays_text(fields.plays_text.as_str().into());
        w.set_show_countdown(fields.show_countdown);
        w.set_badge_clicking(fields.badge_clicking);
        w.set_badge_recording(fields.badge_recording);
        w.set_badge_playback(fields.badge_playback);
        w.set_badge_corner(fields.badge_corner);
    }

    /// What the fields hold now.
    fn fields(&self) -> Fields {
        let w = &self.window;
        Fields {
            tray_delay: w.get_tray_delay(),
            mode: w.get_mode(),
            interval_text: w.get_interval_text().to_string(),
            interval_unit: w.get_interval_unit(),
            jitter: w.get_jitter(),
            button: w.get_button(),
            key_text: w.get_key_text().to_string(),
            position_fixed: w.get_position_fixed(),
            position_x: w.get_position_x(),
            position_y: w.get_position_y(),
            repeat_forever: w.get_repeat_forever(),
            repeat_text: w.get_repeat_text().to_string(),
            output_route: w.get_output_route(),
            hotkeys_route: w.get_hotkeys_route(),
            speed_text: w.get_speed_text().to_string(),
            plays_forever: w.get_plays_forever(),
            plays_text: w.get_plays_text().to_string(),
            show_countdown: w.get_show_countdown(),
            badge_clicking: w.get_badge_clicking(),
            badge_recording: w.get_badge_recording(),
            badge_playback: w.get_badge_playback(),
            badge_corner: w.get_badge_corner(),
        }
    }

    /// Shows the hotkeys: as the desktop reports them where it delivers
    /// them, else as captured since the last save.
    fn show_hotkeys(&self) {
        let w = &self.window;
        let texts = w.global::<Texts>();
        let (not_set, not_reported) = (texts.invoke_not_set(), texts.invoke_not_reported());
        let desktop = self
            .last
            .as_ref()
            .filter(|view| state::desktop_hotkeys(view));
        let shown: [String; 3] = match (desktop, self.state.hotkeys()) {
            (Some(view), _) => [
                &view.hotkeys.toggle_clicking,
                &view.hotkeys.toggle_recording,
                &view.hotkeys.toggle_playback,
            ]
            .map(|hotkey| hotkey_text(hotkey, &not_set, &not_reported)),
            (None, Some(hotkeys)) => [
                hotkeys.toggle_clicking.clone(),
                hotkeys.toggle_recording.clone(),
                hotkeys.toggle_playback.clone(),
            ],
            (None, None) => return,
        };
        let [clicking, recording, playback] = shown;
        w.set_hotkey_clicking(clicking.into());
        w.set_hotkey_recording(recording.into());
        w.set_hotkey_playback(playback.into());
    }

    /// Shows the recordings, keeping those selected that are still
    /// listed. Only for a changed list: new rows reset where the list is
    /// scrolled to.
    fn show_recordings(&mut self, view: &WindowView) {
        let w = &self.window;
        let texts = w.global::<Texts>();
        let no_name = texts.invoke_no_name();
        self.selection.follow(&view.recordings);
        let flags = self.selection.flags(&view.recordings);
        let rows: Vec<RecordingRow> = shelf::lines(&view.recordings, &no_name)
            .into_iter()
            .zip(flags)
            .map(|(line, selected)| RecordingRow {
                text: line.into(),
                selected,
            })
            .collect();
        self.rows.set_vec(rows);
        let more = match view.recordings.more {
            0 => SharedString::new(),
            more => texts.invoke_more_recordings(more.to_string().into()),
        };
        w.set_more_recordings(more);
        w.set_recordings_total(slint_count(total(&view.recordings)));
        self.show_selection();
    }

    /// Changes the selection with `change`, on the list as last shown, and
    /// marks the rows that changed.
    fn select_with(&mut self, change: impl FnOnce(&mut Selection, &RecordingsView)) {
        let Some(view) = &self.last else {
            return;
        };
        change(&mut self.selection, &view.recordings);
        for (at, selected) in self
            .selection
            .flags(&view.recordings)
            .into_iter()
            .enumerate()
        {
            if let Some(row) = self
                .rows
                .row_data(at)
                .filter(|row| row.selected != selected)
            {
                self.rows.set_row_data(at, RecordingRow { selected, ..row });
            }
        }
        self.show_selection();
    }

    /// Shows what the selection allows: the name of the one recording
    /// selected alone in the name field, the actions on one or on several,
    /// and the row the arrows move from. A delete asked about another
    /// selection is called off.
    fn show_selection(&mut self) {
        let recordings = self.last.as_ref().map(|view| &view.recordings);
        let one = self.selection.one().cloned();
        if one != self.named {
            let name = one
                .as_ref()
                .zip(recordings)
                .and_then(|(file, view)| view.listed.iter().find(|l| l.file == *file))
                .and_then(|listed| listed.name.clone())
                .unwrap_or_default();
            self.window.set_recording_name(name.into());
            self.named = one;
        }
        let count = self.selection.files().len();
        self.window
            .set_selected_count(i32::try_from(count).unwrap_or(i32::MAX));
        let cursor = recordings.map_or(-1, |view| self.selection.cursor(view));
        self.window.set_recording_cursor(cursor);
        let total = recordings.map_or(0, total);
        self.state.selected(self.selection.files(), total);
        self.show_question();
    }

    fn rename(&mut self) {
        let name = shelf::sendable_name(&self.window.get_recording_name());
        self.recording(RecordingAction::Rename { name });
    }

    /// Does `action` with the recording selected alone.
    fn recording(&mut self, action: RecordingAction) {
        let Some(file) = self.selection.one().cloned() else {
            return;
        };
        if let Some(request) = self.state.recording(file, action) {
            self.send(&request);
        }
    }

    /// Asks whether to delete the selected recordings.
    fn delete(&mut self) {
        let files = self.selection.files().to_vec();
        let Some(first) = files.first() else {
            return;
        };
        let label = self.last.as_ref().map_or_else(
            || first.label(),
            |view| shelf::label(&view.recordings, first),
        );
        self.state.delete(files, label);
        self.show_question();
    }

    /// Asks whether to delete every recording, listed or not.
    fn delete_all(&mut self) {
        let total = self.last.as_ref().map_or(0, |view| total(&view.recordings));
        self.state.delete_all(total);
        self.show_question();
    }

    fn save(&mut self) {
        let asked = self.state.save(&self.fields());
        self.sent(asked);
    }

    fn capture(&mut self, target: Target) {
        if let Some(request) = self.state.capture(target) {
            self.send(&request);
            self.note(&Note::Capturing(target));
        }
    }

    /// What the capture the window waits on reads, while one does: where
    /// the window captures from its own key events, they go to it.
    #[cfg(windows)]
    pub fn capturing(&self) -> Option<Capture> {
        self.state.capturing()
    }

    /// The window captured `pressed` from its own key events.
    #[cfg(windows)]
    pub fn pressed(&mut self, pressed: Pressed) {
        if let Some(request) = self.state.pressed(pressed) {
            self.send(&request);
        }
    }

    fn configure(&mut self) {
        if let Some(request) = self.state.configure() {
            self.send(&request);
        }
    }

    fn agree(&mut self) {
        let asked = self.state.agree(&self.fields());
        self.sent(asked);
        self.show_question();
    }

    /// Sends a save that could be made, or says which field could not be
    /// read.
    fn sent(&mut self, asked: Option<Result<Request, Unreadable>>) {
        match asked {
            Some(Ok(request)) => self.send(&request),
            Some(Err(field)) => self.note(&Note::Unreadable(field)),
            None => {}
        }
    }

    fn decline(&mut self) {
        self.state.decline();
        self.show_question();
    }

    /// Sends `request`; a daemon that cannot be written to is gone, and so
    /// is the window.
    fn send(&self, request: &Request) {
        if self.link.send(request).is_err() {
            let _ = slint::quit_event_loop();
            return;
        }
        self.show_asking();
    }

    /// Whether the window waits on an answer, and may ask something.
    fn show_asking(&self) {
        self.window.set_waiting(self.state.waiting());
        self.window.set_may_ask(self.state.may_ask());
    }

    /// Says the daemon's messages through `texts`.
    fn said<'a>(&self, texts: &'a Texts<'a>) -> Said<'a> {
        Said {
            texts,
            decimal: self.decimal,
        }
    }

    fn note(&self, note: &Note) {
        let texts = self.window.global::<Texts>();
        self.window.set_message(note_text(&self.said(&texts), note));
        self.window.set_message_is_error(note.is_error());
        match note {
            Note::CapturedKey(code) => self.window.set_key_text(key_text(*code).into()),
            Note::Picked { x, y } => self.fill_picked(*x, *y),
            _ => {}
        }
        self.show_hotkeys();
        self.show_asking();
    }

    fn show_question(&self) {
        let texts = self.window.global::<Texts>();
        let text = match self.state.question() {
            None => SharedString::new(),
            Some(Question::Replace(Stale::Changed)) => texts.invoke_replace_changed(),
            Some(Question::Replace(Stale::Unusable)) => texts.invoke_replace_unusable(),
            Some(Question::DeleteOne(_, label)) => texts.invoke_delete_question(label.into()),
            Some(Question::DeleteSeveral(files)) => {
                let count = i32::try_from(files.as_slice().len()).unwrap_or(i32::MAX);
                texts.invoke_delete_several_question(count)
            }
            Some(Question::DeleteAll(total)) => {
                texts.invoke_delete_all_question(slint_count(*total))
            }
            Some(Question::WithoutModifier(actions)) => {
                let names: Vec<String> = actions
                    .iter()
                    .map(|action| action_name(&texts, *action).to_string())
                    .collect();
                texts.invoke_without_modifier(names.join(", ").into())
            }
        };
        self.window.set_confirm_text(text);
        self.show_asking();
    }
}

/// The text of `note`, in the window's language.
fn note_text(said: &Said<'_>, note: &Note) -> SharedString {
    let texts = said.texts;
    match note {
        Note::Saved => texts.invoke_saved(),
        Note::SavedUnsynced(why) => texts.invoke_saved_unsynced((&**why).into()),
        Note::Busy => texts.invoke_busy(),
        Note::Failed(why) => texts.invoke_failed(said.failure(why)),
        Note::Refused(field, why) => {
            texts.invoke_refused(field_name(texts, *field), said.reason(why))
        }
        Note::Unavailable(why) => texts.invoke_unavailable(said.unavailability(why)),
        Note::Capturing(Target::Key) => texts.invoke_capturing_key(),
        Note::Capturing(Target::Hotkey(_)) => texts.invoke_capturing_hotkey(),
        Note::CapturedHotkey(keys) => texts.invoke_captured_hotkey(keys.into()),
        Note::CapturedKey(code) => texts.invoke_captured_key(key_text(*code).into()),
        Note::CaptureEnded(why) => capture_end(texts, *why),
        Note::Configured => texts.invoke_configured(),
        Note::Picking => texts.invoke_picking(),
        Note::Picked { .. } => texts.invoke_picked(),
        Note::PickCancelled => texts.invoke_capture_cancelled(),
        Note::PickTimedOut => texts.invoke_pick_timed_out(),
        Note::PickFailed(why) => texts.invoke_pick_failed(why.into()),
        Note::Recording(outcome) => said.recording(outcome),
        Note::Deleted(removal) => said.removal(removal),
        Note::NotDeleted(why) => said.not_deleted(why),
        Note::Unreadable(field) => {
            let name = match field {
                Unreadable::Interval => texts.invoke_field_interval(),
                Unreadable::Key => texts.invoke_field_key(),
                Unreadable::Repeat => texts.invoke_field_repeat(),
                Unreadable::Speed => texts.invoke_field_speed(),
                Unreadable::Plays => texts.invoke_field_plays(),
            };
            texts.invoke_unreadable(name)
        }
    }
}

/// How many recordings there are, listed or not.
fn total(view: &RecordingsView) -> u64 {
    u64::try_from(view.listed.len())
        .unwrap_or(u64::MAX)
        .saturating_add(view.more)
}

/// `count` as Slint takes it, at most `i32::MAX`.
fn slint_count(count: u64) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn capture_end(texts: &Texts<'_>, why: CaptureEnd) -> SharedString {
    match why {
        CaptureEnd::Cancelled => texts.invoke_capture_cancelled(),
        CaptureEnd::TimedOut => texts.invoke_capture_timed_out(),
        CaptureEnd::Unnamed => texts.invoke_capture_unnamed(),
        CaptureEnd::Interrupted => texts.invoke_capture_interrupted(),
    }
}

fn field_name(texts: &Texts<'_>, field: Field) -> SharedString {
    match field {
        Field::TrayStartDelay => texts.invoke_field_delay(),
        Field::Interval => texts.invoke_field_interval(),
        Field::Jitter => texts.invoke_field_jitter(),
        Field::Key => texts.invoke_field_key(),
        Field::Repeat => texts.invoke_field_repeat(),
        Field::Hotkey(action) => action_name(texts, action),
        Field::Speed => texts.invoke_field_speed(),
        Field::PlaybackRepeat => texts.invoke_field_plays(),
    }
}

fn action_name(texts: &Texts<'_>, action: Action) -> SharedString {
    match action {
        Action::ToggleClicking => texts.invoke_action_clicking(),
        Action::ToggleRecording => texts.invoke_action_recording(),
        Action::TogglePlayback => texts.invoke_action_playback(),
    }
}

mod picking;
#[cfg(test)]
mod tests;
