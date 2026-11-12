//! Between the window's widgets and its [`State`]: shows what the daemon
//! sends, and turns the user's choices into requests.

use std::cell::RefCell;
use std::rc::Rc;

use auto_clicker_core::config::Field;
use auto_clicker_core::config::Stale;
use auto_clicker_core::hotkeys::Action;
use auto_clicker_core::window::{CaptureEnd, Request, Update, WindowView};
use slint::{ComponentHandle, SharedString};

use crate::link::Link;
use crate::model::{Fields, Unreadable, hotkey_text, key_text};
use crate::said::Said;
use crate::state::{self, Note, Question, State, Target};
use crate::ui::{SettingsWindow, Texts};

/// The window, its link to the daemon and its state.
pub struct App {
    window: SettingsWindow,
    link: Link,
    state: State,
    /// The daemon's last view.
    last: Option<WindowView>,
    /// The decimal separator of the window's language.
    decimal: char,
}

impl App {
    /// Connects the window's callbacks to a new app over `link`.
    pub fn start(window: &SettingsWindow, link: Link, decimal: char) -> Rc<RefCell<App>> {
        let app = Rc::new(RefCell::new(App {
            window: window.clone_strong(),
            link,
            state: State::default(),
            last: None,
            decimal,
        }));
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
        let weak = Rc::downgrade(&app);
        window.on_capture_hotkey(move |action| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().capture(Target::Hotkey(action));
            }
        });
        app
    }

    /// Takes an update from the daemon.
    pub fn update(&mut self, update: Update) {
        match update {
            Update::View(view) => self.view(&view),
            // Shows the window if it is hidden. On Wayland a window cannot
            // bring itself forward or out of the minimised state: that needs
            // an activation token from the tray's click, which neither the
            // menu nor Slint's Wayland-only build can pass on yet.
            Update::Raise => {
                let _ = self.window.show();
            }
        }
    }

    fn view(&mut self, view: &WindowView) {
        let shown = self.state.view(view);
        if let Some(settings) = &shown.fields {
            self.load(&Fields::from_draft(settings, self.decimal));
        }
        self.last = Some(view.clone());
        let w = &self.window;
        w.set_idle(state::idle(view));
        w.set_desktop_hotkeys(state::desktop_hotkeys(view));
        w.set_routes_pending(state::routes_pending(view));
        let texts = w.global::<Texts>();
        let said = self.said(&texts);
        w.set_no_recording(
            view.no_recording
                .as_ref()
                .map(|why| said.no_recording(why))
                .unwrap_or_default(),
        );
        let problems: Vec<SharedString> = view.problems.iter().map(|p| said.problem(p)).collect();
        w.set_problems(problems.join("\n").into());
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
        if let Note::CapturedKey(code) = note {
            self.window.set_key_text(key_text(*code).into());
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

#[cfg(test)]
mod tests;
