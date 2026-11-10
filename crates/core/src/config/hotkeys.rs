//! Checks on `[hotkeys]`: each binding parses, no two actions share one, and
//! none uses the key the clicker presses. The last holds for bindings the
//! program matches itself; where the desktop delivers the hotkeys, its own
//! triggers decide, and the controller checks those.

use toml::Spanned;

use super::HotkeyRoute;
use super::raw::RawHotkeys;
use super::validate::{Invalid, at};
use crate::clicker::ClickerSettings;
use crate::hotkeys::{Binding, Bindings};

/// One action's binding, and where the file sets it (`None` for a default).
struct Placed<'a> {
    binding: Binding,
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
    let placed = [
        (&raw.toggle_clicking, bindings.toggle_clicking),
        (&raw.toggle_recording, bindings.toggle_recording),
        (&raw.toggle_playback, bindings.toggle_playback),
    ]
    .map(|(written, binding)| Placed {
        binding,
        written: written.as_ref(),
    });
    no_duplicates(&placed)?;
    if route == HotkeyRoute::Kernel {
        for one in &placed {
            not_the_clicker_key(one, clicker)?;
        }
    }
    Ok(bindings)
}

fn binding(written: Option<&Spanned<String>>, default: Binding) -> Result<Binding, Invalid> {
    match written {
        Some(value) => Binding::parse(value.get_ref()).map_err(|reason| at(value, reason)),
        None => Ok(default),
    }
}

/// Two actions on one binding would both fire. The error points at a binding
/// the file wrote; at least one of the two is, since the defaults differ.
fn no_duplicates(placed: &[Placed<'_>]) -> Result<(), Invalid> {
    for (index, later) in placed.iter().enumerate() {
        let Some(earlier) = placed[..index]
            .iter()
            .find(|earlier| earlier.binding == later.binding)
        else {
            continue;
        };
        let reason = format!("hotkey {} is bound to two actions", later.binding);
        return Err(match later.written {
            Some(_) => later.invalid(reason),
            None => earlier.invalid(reason),
        });
    }
    Ok(())
}

/// A hotkey on the key the clicker repeats or holds could fire from the
/// clicker itself: a remapper such as keyd re-sends the clicker's keys from
/// its own device, and with a modifier the user happens to hold, they match
/// a binding with modifiers too.
fn not_the_clicker_key(placed: &Placed<'_>, clicker: &ClickerSettings) -> Result<(), Invalid> {
    let binding = placed.binding;
    if Some(binding.key()) == clicker.pressed_key() {
        return Err(placed.invalid(format!(
            "hotkey {binding} uses the key the clicker presses; choose another key"
        )));
    }
    Ok(())
}
