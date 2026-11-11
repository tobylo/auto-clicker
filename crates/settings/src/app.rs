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
use crate::model::{Fields, NotANumber, hotkey_text, key_text};
use crate::state::{self, Note, Question, State, Target};
use crate::ui::{SettingsWindow, Texts};

/// The window, its link to the daemon and its state.
pub struct App {
    window: SettingsWindow,
    link: Link,
    state: State,
}

impl App {
    /// Connects the window's callbacks to a new app over `link`.
    pub fn start(window: &SettingsWindow, link: Link) -> Rc<RefCell<App>> {
        let app = Rc::new(RefCell::new(App {
            window: window.clone_strong(),
            link,
            state: State::default(),
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
            Update::Raise => {
                // Best effort: KWin may leave the window where it is.
                let _ = self.window.show();
            }
        }
    }

    fn view(&mut self, view: &WindowView) {
        let shown = self.state.view(view);
        if let Some(fields) = &shown.fields {
            self.load(fields);
        }
        let w = &self.window;
        w.set_idle(state::idle(view));
        w.set_desktop_hotkeys(state::desktop_hotkeys(view));
        w.set_routes_pending(state::routes_pending(view));
        w.set_no_recording(
            view.no_recording
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_default()
                .into(),
        );
        w.set_problems(view.problems.join("\n").into());
        self.show_keys(Some(view));
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
        w.set_position_fixed(fields.position_fixed);
        w.set_position_x(fields.position_x);
        w.set_position_y(fields.position_y);
        w.set_repeat_forever(fields.repeat_forever);
        w.set_repeat_count(fields.repeat_count);
        w.set_output_route(fields.output_route);
        w.set_hotkeys_route(fields.hotkeys_route);
        w.set_speed_text(fields.speed_text.as_str().into());
        w.set_plays_forever(fields.plays_forever);
        w.set_plays_count(fields.plays_count);
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
            position_fixed: w.get_position_fixed(),
            position_x: w.get_position_x(),
            position_y: w.get_position_y(),
            repeat_forever: w.get_repeat_forever(),
            repeat_count: w.get_repeat_count(),
            output_route: w.get_output_route(),
            hotkeys_route: w.get_hotkeys_route(),
            speed_text: w.get_speed_text().to_string(),
            plays_forever: w.get_plays_forever(),
            plays_count: w.get_plays_count(),
        }
    }

    /// Shows the hotkeys and the key: as captured, or as the desktop
    /// reports them where it delivers the hotkeys.
    fn show_keys(&self, view: Option<&WindowView>) {
        let w = &self.window;
        let texts = w.global::<Texts>();
        let (not_set, not_reported) = (texts.invoke_not_set(), texts.invoke_not_reported());
        let desktop = view.filter(|view| state::desktop_hotkeys(view));
        let shown: [String; 3] = match (desktop, self.state.keys()) {
            (Some(view), _) => [
                &view.hotkeys.toggle_clicking,
                &view.hotkeys.toggle_recording,
                &view.hotkeys.toggle_playback,
            ]
            .map(|hotkey| hotkey_text(hotkey, &not_set, &not_reported)),
            (None, Some(keys)) => [
                keys.hotkeys.toggle_clicking.clone(),
                keys.hotkeys.toggle_recording.clone(),
                keys.hotkeys.toggle_playback.clone(),
            ],
            (None, None) => return,
        };
        let [clicking, recording, playback] = shown;
        w.set_hotkey_clicking(clicking.into());
        w.set_hotkey_recording(recording.into());
        w.set_hotkey_playback(playback.into());
        if let Some(keys) = self.state.keys() {
            w.set_key_name(key_text(keys.clicker.key).into());
        }
    }

    fn save(&mut self) {
        match self.state.save(&self.fields()) {
            Some(Ok(request)) => self.send(&request),
            Some(Err(field)) => self.note(&Note::NotANumber(field)),
            None => {}
        }
    }

    fn capture(&mut self, target: Target) {
        let request = self.state.capture(target);
        self.send(&request);
        self.note(&Note::Capturing(target));
    }

    fn configure(&mut self) {
        let request = self.state.configure();
        self.send(&request);
    }

    fn agree(&mut self) {
        if let Some(request) = self.state.agree() {
            self.send(&request);
        }
        self.show_question();
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
        self.window.set_waiting(self.state.waiting());
    }

    fn note(&self, note: &Note) {
        let texts = self.window.global::<Texts>();
        self.window.set_message(note_text(&texts, note));
        self.window.set_message_is_error(note.is_error());
        self.window.set_waiting(self.state.waiting());
        self.show_keys(None);
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
        self.window.set_waiting(self.state.waiting());
    }
}

/// The text of `note`, in the window's language.
fn note_text(texts: &Texts<'_>, note: &Note) -> SharedString {
    match note {
        Note::Saved => texts.invoke_saved(),
        Note::SavedUnsynced(why) => texts.invoke_saved_unsynced(why.into()),
        Note::Busy => texts.invoke_busy(),
        Note::Failed(why) => texts.invoke_failed(why.into()),
        Note::Refused(field, why) => texts.invoke_refused(field_name(texts, *field), why.into()),
        Note::Unavailable(why) => texts.invoke_unavailable(why.into()),
        Note::Capturing(Target::Key) => texts.invoke_capturing_key(),
        Note::Capturing(Target::Hotkey(_)) => texts.invoke_capturing_hotkey(),
        Note::CaptureEnded(why) => capture_end(texts, *why),
        Note::Configured => texts.invoke_configured(),
        Note::NotANumber(field) => {
            let name = match field {
                NotANumber::Interval => texts.invoke_field_interval(),
                NotANumber::Speed => texts.invoke_field_speed(),
            };
            texts.invoke_not_a_number(name)
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
