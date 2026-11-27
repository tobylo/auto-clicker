//! Hotkeys a system delivers for bindings the program registered with it
//! (Windows' `RegisterHotKey`, M5 brief decision 9), and the raw keyboard
//! input read beside them, only while it is needed (invariant N5).
//!
//! The system says when a hotkey fires, never when its keys come up, so the
//! wait for a hotkey's modifiers reads the keyboard: from the activation
//! until the modifiers are up, or until the controller no longer waits. And
//! a game in front can switch the system's hotkeys off (`RIDEV_NOHOTKEYS`),
//! so while a run is active the keyboard is also matched against the
//! combinations of the hotkeys that stop it. One registration serves both:
//! it is made while either needs it and removed once neither does.
//!
//! One press can arrive both ways and counts once. A stop counted from raw
//! input waits for its hotkey until the press's key comes up, and keeps
//! the keyboard read until then, so its hotkey is known for the same press
//! however late it comes. A hotkey counted first needs no pair: its key is
//! then down in what is followed (from raw input, or from the key state
//! read at the hotkey), so a raw down of it after is a repeat. (On Windows
//! a hotkey that fires never shows its key's press in raw input, and the
//! backend hands in the raw input queued before a hotkey first.)
//!
//! Everything here is plain logic over key codes; the backend does the
//! reading through [`Keyboard`], and its own events never get here (the
//! backend drops what carries the program's marker).

use std::io;

use super::matcher::MODIFIER_CODES;
use super::{Action, Binding, HotkeyEvent, Modifier, Modifiers};
use crate::input::Key;

/// What the controller needs read from the keyboard now. It sends this to
/// the reader whenever it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watching {
    /// A start waits for its hotkey's modifiers to come up.
    pub release: bool,
    /// The hotkeys that stop the run that is going, if one is.
    pub stops: Vec<Action>,
}

/// The raw keyboard input a backend reads, and the system's key state.
pub trait Keyboard {
    /// Starts reading raw keyboard input.
    ///
    /// # Errors
    /// The system refused the registration.
    fn start(&mut self) -> io::Result<()>;
    /// Stops reading it.
    fn stop(&mut self);
    /// Whether the user holds the key with this code down now, as the
    /// system's own key state says: never a key the program itself holds.
    /// One the state still shows after the program let go of it is put
    /// right by [`Registered::own_release`].
    fn held(&self, code: u16) -> bool;
}

/// A hotkey the system took.
#[derive(Debug)]
struct Taken {
    action: Action,
    binding: Binding,
    /// The codes of every key that sets it off: its own, and those the
    /// system takes for it.
    keys: Vec<u16>,
}

/// The registered hotkeys and what the keyboard is read for.
#[derive(Debug)]
pub struct Registered {
    /// The hotkeys the system took; a refused one is neither delivered nor
    /// matched.
    hotkeys: Vec<Taken>,
    watching: Watching,
    /// Whether raw keyboard input is being read.
    reading: bool,
    /// The keys down while reading, among the modifiers and the hotkeys'
    /// keys: nothing else is kept.
    held: Vec<u16>,
    /// Those of `held` known only from the key state, which may have
    /// counted a key the program had just let go of: no raw event of the
    /// user's has confirmed them yet.
    seeded: Vec<u16>,
    /// An activation's modifiers have not been seen coming up yet: a
    /// [`HotkeyEvent::ModifiersReleased`] or [`HotkeyEvent::ReleaseLost`]
    /// is owed.
    owed: bool,
    /// The modifiers were reported up since the last activation.
    released: bool,
    /// Stops raw input counted whose hotkey may still come for the same
    /// press.
    unpaired: Vec<Action>,
}

impl Registered {
    /// The hotkeys the system took, each with its binding, and set off by
    /// the keys `keys_of` gives for it (its own key among them, as
    /// [`super::Triggers::bound`] takes them). Nothing is read yet.
    pub fn new(
        hotkeys: Vec<(Action, Binding)>,
        keys_of: impl Fn(Binding) -> Vec<Key>,
    ) -> Registered {
        let hotkeys = hotkeys
            .into_iter()
            .map(|(action, binding)| {
                let mut keys: Vec<u16> = keys_of(binding).iter().map(|key| key.code()).collect();
                if !keys.contains(&binding.key().code()) {
                    keys.push(binding.key().code());
                }
                Taken {
                    action,
                    binding,
                    keys,
                }
            })
            .collect();
        Registered {
            hotkeys,
            watching: Watching::default(),
            reading: false,
            held: Vec::new(),
            seeded: Vec::new(),
            owed: false,
            released: false,
            unpaired: Vec::new(),
        }
    }

    /// Whether raw keyboard input is being read.
    pub fn reading(&self) -> bool {
        self.reading
    }

    /// The system delivered `action`'s hotkey: it fires, unless it is the
    /// other half of a press already counted, and its modifiers are waited
    /// for. The events to send, in order.
    pub fn hotkey(&mut self, action: Action, keyboard: &mut impl Keyboard) -> Vec<HotkeyEvent> {
        if !self.took(action) || self.paired(action) {
            return Vec::new();
        }
        let mut events = vec![HotkeyEvent::Fired(action)];
        self.activated();
        if self.reading {
            // Raw events of this very press may still be queued behind the
            // hotkey: what the system's state says is down counts too. Its
            // key among them, should it show: a raw down of it after is then
            // a repeat, never a second press.
            self.seed(keyboard);
        }
        self.settle(keyboard, &mut events);
        events
    }

    /// A key went down or came up, read from raw input. Only keys read for
    /// a reason are kept; the others pass unseen.
    pub fn key(&mut self, code: u16, down: bool, keyboard: &mut impl Keyboard) -> Vec<HotkeyEvent> {
        let mut events = Vec::new();
        if !self.reading || !self.tracked(code) {
            return events;
        }
        let was_down = self.held.contains(&code);
        // The user's own event: the key state's word on it is settled.
        self.seeded.retain(|&seeded| seeded != code);
        if down && !was_down {
            self.held.push(code);
            self.pressed(code, &mut events);
        } else if !down {
            self.held.retain(|&held| held != code);
            self.forget_seeded_aliases(code, keyboard);
            let hotkeys = &self.hotkeys;
            self.unpaired.retain(|action| {
                !hotkeys
                    .iter()
                    .any(|taken| taken.action == *action && taken.keys.contains(&code))
            });
        }
        self.settle(keyboard, &mut events);
        events
    }

    /// The program let go of a key it held, read from raw input: should the
    /// key state alone have counted it as the user's, it is up now. A key
    /// the user's own press put down stays down. Nothing fires.
    pub fn own_release(&mut self, code: u16, keyboard: &mut impl Keyboard) -> Vec<HotkeyEvent> {
        let mut events = Vec::new();
        if self.reading {
            if self.seeded.contains(&code) {
                self.seeded.retain(|&seeded| seeded != code);
                self.held.retain(|&held| held != code);
            }
            // Its aliases may have been seeded from its state while it was
            // not (the program held it then).
            self.forget_seeded_aliases(code, keyboard);
            self.settle(keyboard, &mut events);
        }
        events
    }

    /// The controller now needs `watching`. A wait it stopped waiting for
    /// ends; one it waits for and the keyboard no longer follows (a newer
    /// activation's wait ended on an older answer) is taken up again with
    /// a fresh read, unless its release was already reported.
    pub fn watch(&mut self, watching: Watching, keyboard: &mut impl Keyboard) -> Vec<HotkeyEvent> {
        match (self.watching.release, watching.release) {
            (true, false) => self.owed = false,
            (false, true) => self.wait_again(keyboard),
            _ => {}
        }
        self.watching = watching;
        let mut events = Vec::new();
        self.settle(keyboard, &mut events);
        events
    }

    /// The session was locked, unlocked or switched, or the input desktop
    /// changed: what the keyboard did meanwhile went to another desktop,
    /// releases included. Every key followed is forgotten, with any press
    /// still waiting for its other half, and taken afresh from the key
    /// state while reading goes on. A wait for the modifiers is lost: the
    /// key state reads every key up while another desktop has the input.
    pub fn session_changed(&mut self, keyboard: &mut impl Keyboard) -> Vec<HotkeyEvent> {
        self.held.clear();
        self.seeded.clear();
        self.unpaired.clear();
        let mut events = Vec::new();
        if self.owed {
            self.owed = false;
            events.push(HotkeyEvent::ReleaseLost);
        }
        if self.reading {
            self.seed(keyboard);
        }
        self.settle(keyboard, &mut events);
        events
    }

    /// Stops reading whatever it read for: the reader is gone, and with
    /// it any press still waiting for its other half.
    pub fn end(&mut self, keyboard: &mut impl Keyboard) {
        self.owed = false;
        self.watching = Watching::default();
        self.unpaired.clear();
        let mut events = Vec::new();
        self.settle(keyboard, &mut events);
    }

    /// The controller waits for a release the keyboard no longer follows,
    /// unless it was reported already.
    fn wait_again(&mut self, keyboard: &impl Keyboard) {
        if self.owed || self.released {
            return;
        }
        self.owed = true;
        if self.reading {
            self.seed(keyboard);
        }
    }

    /// Whether the system took `action`'s hotkey.
    fn took(&self, action: Action) -> bool {
        self.hotkeys.iter().any(|taken| taken.action == action)
    }

    /// Whether `action` was counted from raw input and waits for its
    /// hotkey; the pair is used up.
    fn paired(&mut self, action: Action) -> bool {
        let index = self.unpaired.iter().position(|&counted| counted == action);
        index
            .map(|index| self.unpaired.swap_remove(index))
            .is_some()
    }

    fn activated(&mut self) {
        self.owed = true;
        self.released = false;
    }

    /// A key went down that was up: a stop combination fires.
    fn pressed(&mut self, code: u16, events: &mut Vec<HotkeyEvent>) {
        let modifiers = self.modifiers();
        let stop = self.watching.stops.iter().copied().find(|&action| {
            self.hotkeys.iter().any(|taken| {
                taken.action == action
                    && taken.keys.contains(&code)
                    && taken.binding.modifiers() == modifiers
            })
        });
        let Some(action) = stop else {
            return;
        };
        events.push(HotkeyEvent::Fired(action));
        self.activated();
        self.unpaired.push(action);
    }

    /// Whether `code` is read: a modifier, or a hotkey's key.
    fn tracked(&self, code: u16) -> bool {
        MODIFIER_CODES.contains(&code)
            || self.hotkeys.iter().any(|taken| taken.keys.contains(&code))
    }

    /// `code` came up. A key the system takes for the same one
    /// may share its key state (keypad Enter is Enter's): a seed of one that
    /// no raw event confirmed, and that the key state now says is up, was
    /// this key. One whose own state is still down (Z beside Y on another
    /// layout) stays.
    fn forget_seeded_aliases(&mut self, code: u16, keyboard: &impl Keyboard) {
        let up: Vec<u16> = self
            .hotkeys
            .iter()
            .filter(|taken| taken.keys.contains(&code))
            .flat_map(|taken| taken.keys.iter().copied())
            .filter(|&alias| self.seeded.contains(&alias) && !keyboard.held(alias))
            .collect();
        self.held.retain(|held| !up.contains(held));
        self.seeded.retain(|seeded| !up.contains(seeded));
    }

    /// Adds what the system's key state says is down.
    fn seed(&mut self, keyboard: &impl Keyboard) {
        let hotkeys = self
            .hotkeys
            .iter()
            .flat_map(|taken| taken.keys.iter().copied());
        for code in MODIFIER_CODES.into_iter().chain(hotkeys) {
            if keyboard.held(code) && !self.held.contains(&code) {
                self.held.push(code);
                self.seeded.push(code);
            }
        }
    }

    fn modifiers(&self) -> Modifiers {
        self.held
            .iter()
            .filter_map(|&code| Modifier::of_code(code))
            .collect()
    }

    /// Whether anything needs the keyboard read: an owed release, the
    /// run's stop matching, or a press raw input counted whose hotkey may
    /// still come. The last lasts while its key is down, so a hotkey that
    /// arrives after the stop it duplicates, behind the controller's
    /// answer to it, is still known for the same press.
    fn wanted(&self) -> bool {
        self.owed || !self.watching.stops.is_empty() || !self.unpaired.is_empty()
    }

    /// Starts reading when something needs it and stops once nothing
    /// does, reporting an owed release once the modifiers are up.
    fn settle(&mut self, keyboard: &mut impl Keyboard, events: &mut Vec<HotkeyEvent>) {
        if self.wanted() && !self.reading {
            self.start(keyboard, events);
        }
        if self.reading {
            self.release_if_up(events);
        }
        if !self.wanted() && self.reading {
            keyboard.stop();
            self.reading = false;
            self.held.clear();
            self.seeded.clear();
            self.unpaired.clear();
        }
    }

    /// Starts reading, from what the key state says is down; an owed
    /// release is lost when it cannot.
    fn start(&mut self, keyboard: &mut impl Keyboard, events: &mut Vec<HotkeyEvent>) {
        if keyboard.start().is_ok() {
            self.reading = true;
            self.held.clear();
            self.seeded.clear();
            self.seed(keyboard);
        } else if self.owed {
            self.owed = false;
            events.push(HotkeyEvent::ReleaseLost);
        }
    }

    /// Reports an owed release once no modifier is down.
    fn release_if_up(&mut self, events: &mut Vec<HotkeyEvent>) {
        if self.owed && self.modifiers() == Modifiers::NONE {
            self.owed = false;
            self.released = true;
            events.push(HotkeyEvent::ModifiersReleased);
        }
    }
}

#[cfg(test)]
mod tests;
