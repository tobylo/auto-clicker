//! Gathering the outputs' updates into one whole layout (brief W3).
//!
//! The compositor sends each output's update on its own, ended by its
//! `done`, and one change can touch several outputs (a scale change moves
//! the output beside it). So after any update the connection asks for a
//! round trip (`wl_display.sync`), and the layout is read only once its
//! answer arrives with nothing new since and no output's update unfinished:
//! by then every update the compositor sent for the change is in.
//! Until then the last whole layout stands, and an update that changes no
//! place or size publishes nothing.
//!
//! No Wayland type lives here: the connection feeds [`Gather`] what it
//! heard, so the order of events is tested without a compositor.

use std::collections::{BTreeMap, BTreeSet};

use auto_clicker_core::recording::{Layout, Output, Rect};

/// What the compositor said about one output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    /// Its logical top-left corner (xdg-output).
    Position { x: i32, y: i32 },
    /// Its logical size (xdg-output).
    Size { width: i32, height: i32 },
    /// Its current mode, in physical pixels.
    Mode { width: i32, height: i32 },
    /// Whether it is turned a quarter: its mode's width is then its height.
    Turned(bool),
    /// Its whole-number scale (`wl_output.scale`).
    Scale(i32),
    /// `wl_output.done`, the end of the output's update; with xdg-output
    /// 3, which sends no `done` of its own, of its xdg-output's too.
    OutputDone { with_xdg: bool },
    /// `zxdg_output_v1.done`, the end of its xdg-output's update (xdg-output
    /// 1 and 2).
    XdgDone,
}

/// One output as heard so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Screen {
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    mode: Option<(i32, i32)>,
    turned: bool,
    whole_scale: Option<i32>,
    /// Something was heard on `wl_output` since its `done`.
    output_updating: bool,
    /// Something was heard on its xdg-output since that update's end.
    xdg_updating: bool,
}

impl Screen {
    /// Notes `heard`; whether it moved the output or changed its size.
    fn hear(&mut self, heard: Heard) -> bool {
        let before = (self.position, self.size);
        match heard {
            Heard::Position { x, y } => self.position = Some((x, y)),
            Heard::Size { width, height } => self.size = Some((width, height)),
            Heard::Mode { width, height } => self.mode = Some((width, height)),
            Heard::Turned(turned) => self.turned = turned,
            Heard::Scale(scale) => self.whole_scale = Some(scale),
            Heard::OutputDone { with_xdg } => {
                self.output_updating = false;
                self.xdg_updating &= !with_xdg;
                return false;
            }
            Heard::XdgDone => {
                self.xdg_updating = false;
                return false;
            }
        }
        match heard {
            Heard::Position { .. } | Heard::Size { .. } => self.xdg_updating = true,
            _ => self.output_updating = true,
        }
        (self.position, self.size) != before
    }

    /// Whether an update on either object has not ended.
    fn updating(&self) -> bool {
        self.output_updating || self.xdg_updating
    }
}

/// Where the layout's (0, 0) lies in the compositor's own coordinates: the
/// corner of the outputs' bounding box, by which the layout was moved
/// (brief W3). A global position less it is a position on the layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Corner {
    pub x: i64,
    pub y: i64,
}

/// What a round trip settled on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Settled {
    /// The layout; `None` when it is not whole or not valid: unknown.
    pub(crate) layout: Option<Layout>,
    /// Where the layout was moved from; (0, 0) without one.
    pub(crate) corner: Corner,
    /// An output came, went, moved or changed size since the last one, even
    /// if it is back where it was: every such change refuses a pick begun
    /// before it.
    pub(crate) moved: bool,
}

/// The outputs, and where the round trip after their last update stands.
#[derive(Debug, Default)]
pub(crate) struct Gather {
    /// By the name of their global.
    screens: BTreeMap<u32, Screen>,
    /// Outputs too old to say when an update ends (`wl_output` 1): while
    /// any is offered, the layout is unknown.
    refused: BTreeSet<u32>,
    /// Something was heard since the last round trip was asked for.
    changed: bool,
    /// A round trip was asked for and has not come back.
    syncing: bool,
    /// See [`Settled::moved`]; since the last settled round trip.
    moved: bool,
}

impl Gather {
    /// The output `name` was bound.
    pub(crate) fn added(&mut self, name: u32) {
        self.screens.insert(name, Screen::default());
        self.came_or_went();
    }

    /// The output `name` is offered but too old to be bound.
    pub(crate) fn refused(&mut self, name: u32) {
        self.refused.insert(name);
        self.came_or_went();
    }

    /// The output `name` went.
    pub(crate) fn removed(&mut self, name: u32) {
        let bound = self.screens.remove(&name).is_some();
        if self.refused.remove(&name) || bound {
            self.came_or_went();
        }
    }

    fn came_or_went(&mut self) {
        self.changed = true;
        self.moved = true;
    }

    /// The compositor said `heard` about the output `name`.
    pub(crate) fn heard(&mut self, name: u32, heard: Heard) {
        let Some(screen) = self.screens.get_mut(&name) else {
            return;
        };
        self.moved |= screen.hear(heard);
        self.changed = true;
    }

    /// Asks for the first layout even when nothing is bound.
    pub(crate) fn start(&mut self) {
        self.changed = true;
    }

    /// Whether to ask for a round trip now: something was heard and none
    /// is on its way.
    pub(crate) fn wants_sync(&mut self) -> bool {
        if !self.changed || self.syncing {
            return false;
        }
        self.changed = false;
        self.syncing = true;
        true
    }

    /// The round trip came back: the layout now, or `None` when more was
    /// heard meanwhile and another round trip is due, or an output's update
    /// has not ended (its `done` asks for the next). A layout that is not
    /// whole or not valid is unknown.
    pub(crate) fn synced(&mut self) -> Option<Settled> {
        self.syncing = false;
        let updating = self.screens.values().any(Screen::updating);
        if self.changed || updating {
            return None;
        }
        let (layout, corner) = self
            .layout()
            .map_or((None, Corner::default()), |(layout, corner)| {
                (Some(layout), corner)
            });
        Some(Settled {
            layout,
            corner,
            moved: std::mem::take(&mut self.moved),
        })
    }

    /// The outputs as one layout, moved so its bounding box starts at
    /// (0, 0), and the corner it was moved from; `None` when an output's
    /// place or size is still missing, or they make no layout
    /// ([`Layout::new`]).
    fn layout(&self) -> Option<(Layout, Corner)> {
        if !self.refused.is_empty() {
            return None;
        }
        let placed: Vec<Placed> = self
            .screens
            .values()
            .map(Placed::of)
            .collect::<Option<_>>()?;
        let left = placed.iter().map(|screen| screen.x).min()?;
        let top = placed.iter().map(|screen| screen.y).min()?;
        let outputs = placed
            .iter()
            .map(|screen| screen.output(left, top))
            .collect::<Option<_>>()?;
        let corner = Corner { x: left, y: top };
        Layout::new(outputs).ok().map(|layout| (layout, corner))
    }
}

/// An output with its place and size known.
struct Placed {
    x: i64,
    y: i64,
    width: u32,
    height: u32,
    scale_milli: u32,
}

impl Placed {
    fn of(screen: &Screen) -> Option<Placed> {
        let (x, y) = screen.position?;
        let (width, height) = screen.size?;
        let width = u32::try_from(width).ok()?;
        let height = u32::try_from(height).ok()?;
        Some(Placed {
            x: i64::from(x),
            y: i64::from(y),
            width,
            height,
            scale_milli: scale_milli(screen, width)?,
        })
    }

    /// The output moved by (`left`, `top`), the bounding box's corner.
    fn output(&self, left: i64, top: i64) -> Option<Output> {
        let rect = Rect {
            x: u32::try_from(self.x.checked_sub(left)?).ok()?,
            y: u32::try_from(self.y.checked_sub(top)?).ok()?,
            width: self.width,
            height: self.height,
        };
        Output::new(rect, self.scale_milli).ok()
    }
}

/// The scale in thousandths: the current mode's width over the logical
/// width (the mode's height when the output is turned a quarter), rounded;
/// the whole-number scale without a mode, and 1 without either. Stored,
/// never compared (R03 decision 6).
fn scale_milli(screen: &Screen, logical_width: u32) -> Option<u32> {
    let physical = match screen.mode {
        Some((width, height)) => {
            if screen.turned {
                height
            } else {
                width
            }
        }
        None => {
            let whole = screen.whole_scale.unwrap_or(1);
            return u32::try_from(whole).ok()?.checked_mul(1000);
        }
    };
    let physical = u64::try_from(physical).ok()?;
    let logical = u64::from(logical_width);
    let rounded = physical
        .checked_mul(1000)?
        .checked_add(logical / 2)?
        .checked_div(logical)?;
    u32::try_from(rounded).ok()
}

#[cfg(test)]
mod tests;
