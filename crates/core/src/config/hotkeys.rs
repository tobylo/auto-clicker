//! Checks on `[hotkeys]`: each binding parses, no two actions share one, and
//! none uses the key the clicker presses (see [`check::on_the_clicker_key`]).
//! The last holds for bindings the program may match itself (`kernel`, and
//! `auto`, which may pick it); where the desktop delivers the hotkeys, its
//! own triggers decide, and the controller checks those.

use toml::Spanned;

use super::HotkeyRoute;
use super::check;
use super::raw::RawHotkeys;
use super::validate::{Invalid, at};
use crate::clicker::ClickerSettings;
use crate::hotkeys::{Action, Binding, Bindings};

/// Where the file sets an action's binding (`None` for a default).
struct Placed<'a> {
    written: Option<&'a Spanned<String>>,
}

impl Placed<'_> {
    fn invalid(&self, reason: String) -> Invalid {
        match self.written {
            Some(value) => at(value, reason),
            None => Invalid { span: None, reason },
        }
    }
}

pub(super) fn hotkeys(
    raw: &RawHotkeys,
    clicker: &ClickerSettings,
    route: HotkeyRoute,
) -> Result<Bindings, Invalid> {
    let defaults = Bindings::default();
    let bindings = Bindings {
        toggle_clicking: binding(raw.toggle_clicking.as_ref(), defaults.toggle_clicking)?,
        toggle_recording: binding(raw.toggle_recording.as_ref(), defaults.toggle_recording)?,
        toggle_playback: binding(raw.toggle_playback.as_ref(), defaults.toggle_playback)?,
    };
    let placed = |action: Action| Placed {
        written: match action {
            Action::ToggleClicking => raw.toggle_clicking.as_ref(),
            Action::ToggleRecording => raw.toggle_recording.as_ref(),
            Action::TogglePlayback => raw.toggle_playback.as_ref(),
        },
    };
    if let Some((earlier, later)) = check::duplicate(&bindings) {
        // At least one of the two is written, since the defaults differ.
        let reason = check::bound_twice(bindings.binding(later));
        let later = placed(later);
        return Err(match later.written {
            Some(_) => later.invalid(reason),
            None => placed(earlier).invalid(reason),
        });
    }
    if route.may_read_keyboards()
        && let Some((action, reason)) = check::on_the_clicker_key(&bindings, clicker)
    {
        return Err(placed(action).invalid(reason));
    }
    Ok(bindings)
}

fn binding(written: Option<&Spanned<String>>, default: Binding) -> Result<Binding, Invalid> {
    match written {
        Some(value) => Binding::parse(value.get_ref()).map_err(|reason| at(value, reason)),
        None => Ok(default),
    }
}
