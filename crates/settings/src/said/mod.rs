//! The daemon's messages in the window's language, through the window's
//! catalogue: one function per family of messages. Text from another
//! program, quoted in a message, is shown as it is.

use auto_clicker_core::config::Reason;
use auto_clicker_core::controller::{Blocked, NoRecording};
use auto_clicker_core::hotkeys::Unavailable;
use auto_clicker_core::message::{ConfigProblem, Failure, InputFailure, Problem, Unavailability};
use auto_clicker_core::playback::Speed;
use slint::SharedString;

use crate::ui::Texts;

mod notice;

/// Shows numbers with the decimal separator of the window's language.
pub struct Said<'a> {
    pub texts: &'a Texts<'a>,
    pub decimal: char,
}

/// `text` as Slint takes it.
fn s(text: impl ToString) -> SharedString {
    text.to_string().into()
}

impl Said<'_> {
    pub fn problem(&self, problem: &Problem) -> SharedString {
        let t = self.texts;
        match problem {
            Problem::Config(problem) => self.config(problem),
            Problem::Hotkeys(why) => self.unavailable(why),
            Problem::Blocked(why) => t.invoke_problem_clicking(self.blocked(why)),
            Problem::Input(why) => t.invoke_problem_clicking(self.input(why)),
            Problem::Unlisted(why) => t.invoke_problem_unlisted(s(why)),
            Problem::CurrentUnread(why) => t.invoke_problem_current_unread(s(why)),
            Problem::CurrentUnkept(why) => t.invoke_problem_current_unkept(s(why)),
        }
    }

    fn config(&self, problem: &ConfigProblem) -> SharedString {
        let t = self.texts;
        match problem {
            ConfigProblem::NoDirectory => t.invoke_config_no_directory(),
            ConfigProblem::Unreadable { file, why } => t.invoke_config_unreadable(s(file), s(why)),
            ConfigProblem::Invalid {
                file,
                line: Some(line),
                why,
            } => t.invoke_config_invalid_at(s(file), s(line), s(why)),
            ConfigProblem::Invalid {
                file,
                line: None,
                why,
            } => t.invoke_config_invalid(s(file), s(why)),
        }
    }

    pub(super) fn blocked(&self, why: &Blocked) -> SharedString {
        let t = self.texts;
        match why {
            Blocked::Locked => t.invoke_blocked_locked(),
            Blocked::Inactive => t.invoke_blocked_inactive(),
            Blocked::Sleeping => t.invoke_blocked_sleeping(),
            Blocked::Unknown(why) => t.invoke_blocked_unknown(s(why)),
        }
    }

    pub(super) fn input(&self, why: &InputFailure) -> SharedString {
        let t = self.texts;
        match why {
            InputFailure::NoPermission(e) => t.invoke_input_no_permission(s(e)),
            InputFailure::UinputMissing(e) => t.invoke_input_uinput_missing(s(e)),
            InputFailure::CreateDevice(e) => t.invoke_input_create_device(s(e)),
            InputFailure::Write(e) => t.invoke_input_write(s(e)),
            InputFailure::PortalUnavailable(why) => t.invoke_input_portal_unavailable(s(why)),
            InputFailure::NotAllowed => t.invoke_input_not_allowed(),
            InputFailure::AllowedNow => t.invoke_input_allowed_now(),
            InputFailure::Portal(why) => t.invoke_input_portal(s(why)),
            InputFailure::SessionLost(why) => t.invoke_input_session_lost(s(why)),
        }
    }

    pub(super) fn unavailable(&self, why: &Unavailable) -> SharedString {
        let t = self.texts;
        match why {
            Unavailable::NoKeyboard => t.invoke_hotkeys_no_keyboard(),
            Unavailable::NoPermission => t.invoke_hotkeys_no_permission(),
            Unavailable::ReadFailed(e) => t.invoke_hotkeys_read_failed(s(e)),
            Unavailable::WatchFailed(e) => t.invoke_hotkeys_watch_failed(s(e)),
            Unavailable::Desktop(why) => t.invoke_hotkeys_desktop(s(why)),
        }
    }

    pub fn no_recording(&self, why: &NoRecording) -> SharedString {
        match why {
            NoRecording::DesktopHotkeys => self.texts.invoke_no_recording_desktop(),
            NoRecording::Keyboards(why) => self.unavailable(why),
        }
    }

    pub fn failure(&self, why: &Failure) -> SharedString {
        let t = self.texts;
        match why {
            Failure::NoConfigDirectory => t.invoke_failure_no_config_directory(),
            Failure::Write(why) => t.invoke_failure_write(s(why)),
            Failure::BadToken => t.invoke_failure_bad_token(),
            Failure::Desktop(why) => s(why),
            Failure::NotOpened => t.invoke_failure_not_opened(),
            Failure::Hotkeys(why) => self.unavailable(why),
        }
    }

    pub fn unavailability(&self, why: &Unavailability) -> SharedString {
        let t = self.texts;
        match why {
            Unavailability::DesktopDelivers => t.invoke_desktop_delivers(),
            Unavailability::Blocked(why) => self.blocked(why),
            Unavailability::ReadsKeyboards => t.invoke_reads_keyboards(),
            Unavailability::NoShortcuts => t.invoke_no_shortcuts(),
            Unavailability::Hotkeys(why) => self.unavailable(why),
            Unavailability::ScreensUnknown => t.invoke_screens_unknown(),
            Unavailability::ScreensChanged => t.invoke_screens_changed(),
        }
    }

    pub fn reason(&self, why: &Reason) -> SharedString {
        let t = self.texts;
        match why {
            Reason::TrayStartDelay { max } => t.invoke_reason_tray_start_delay(s(max)),
            Reason::Interval { floor, max, double } => {
                let ask = if *double {
                    Texts::invoke_reason_interval_double
                } else {
                    Texts::invoke_reason_interval
                };
                ask(t, s(floor), s(max))
            }
            Reason::Rate { floor, max, double } => self.rate(*floor, *max, *double),
            Reason::Jitter { max } => t.invoke_reason_jitter(s(max)),
            Reason::Speed => {
                t.invoke_reason_speed(self.number(Speed::SLOWEST), self.number(Speed::FASTEST))
            }
            Reason::KeyCode { code } => t.invoke_reason_key_code(s(code)),
            Reason::Count { count } => t.invoke_reason_count(s(count), s(u32::MAX)),
            Reason::BoundTwice(hotkey) => t.invoke_reason_bound_twice(s(hotkey)),
            Reason::OnClickerKey(hotkey) => t.invoke_reason_on_clicker_key(s(hotkey)),
            Reason::Binding(why) => s(why),
            Reason::DesktopChooses => t.invoke_desktop_delivers(),
        }
    }

    fn rate(&self, floor: u64, max: u64, double: bool) -> SharedString {
        let (least, most) = Reason::rates(floor, max);
        let ask = if double {
            Texts::invoke_reason_rate_double
        } else {
            Texts::invoke_reason_rate
        };
        ask(
            self.texts,
            self.number(format!("{least:.6}")),
            self.number(most),
        )
    }

    /// `number` with the window's decimal separator.
    fn number(&self, number: impl ToString) -> SharedString {
        s(number.to_string().replace('.', &self.decimal.to_string()))
    }
}
