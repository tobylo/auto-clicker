//! The on-screen display's objects on its own connection (O01's O4): a
//! layer-shell surface per screen on the overlay layer, in the middle for
//! the countdown or anchored to a corner for the badge, asking for no
//! keyboard and with an empty input region (O6), each drawn into a buffer
//! in shared memory at the screen's fractional scale.
//!
//! The layout connection only notes whether the globals are offered, and
//! binds none of them (O3). On the display's connection the outputs and
//! their sizes come from [`Gather`], one screen at a time: a screen whose
//! update has not ended waits for it, and one that cannot show keeps its
//! reason ([`Missing`]) while the others show.
//!
//! [`Gather`]: crate::gather::Gather

use std::collections::BTreeMap;

use auto_clicker_core::badge::{Badge, Mark};
use auto_clicker_core::config::Corner;
use auto_clicker_core::digits::{Label, Rgb};
use auto_clicker_core::recording::MAX_OUTPUTS;
use tokio::time::Instant;
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{
    Layer, ZwlrLayerShellV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
};

use super::{Arrived, Client, Global, Link};
use crate::display::fit::{Fit, MARGIN, Missing, Place, WHOLE};
use crate::display::{ANSWER, NAMESPACE, paint, shm};

pub(crate) const COMPOSITOR: &str = "wl_compositor";
pub(crate) const SHM: &str = "wl_shm";
pub(crate) const LAYER_SHELL: &str = "zwlr_layer_shell_v1";
pub(crate) const VIEWPORTER: &str = "wp_viewporter";
pub(crate) const FRACTIONS: &str = "wp_fractional_scale_manager_v1";

/// The most buffers a screen holds that the compositor has not released:
/// the one it shows and the next. A screen with both waits for a release
/// before it draws again, so a compositor that keeps them holds no more.
const LIVE: usize = 2;

/// `wl_surface.damage_buffer` came in version 4.
const OLDEST_COMPOSITOR: u32 = 4;
/// The newest versions the client knows; every request it makes is in
/// these.
const COMPOSITOR_VERSION: u32 = 4;
const SHELL_VERSION: u32 = 4;
const VERSION_1: u32 = 1;

/// The globals the display needs, as offered.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Offered {
    compositor: Option<Global>,
    shm: Option<Global>,
    shell: Option<Global>,
    viewporter: Option<Global>,
    fractions: Option<Global>,
}

impl Offered {
    /// Notes the global `interface`, if it is one of these.
    pub(crate) fn note(&mut self, interface: &str, global: Global) {
        let slot = match interface {
            COMPOSITOR => &mut self.compositor,
            SHM => &mut self.shm,
            LAYER_SHELL => &mut self.shell,
            VIEWPORTER => &mut self.viewporter,
            FRACTIONS => &mut self.fractions,
            _ => return,
        };
        slot.get_or_insert(global);
    }

    /// The global `name` went: one withdrawn while the connection's first
    /// globals are heard is not used.
    pub(crate) fn removed(&mut self, name: u32) {
        let slots = [
            &mut self.compositor,
            &mut self.shm,
            &mut self.shell,
            &mut self.viewporter,
            &mut self.fractions,
        ];
        for slot in slots {
            if slot.is_some_and(|global| global.name == name) {
                *slot = None;
            }
        }
    }

    /// Whether the display can show: layer-shell, shared memory and
    /// surfaces new enough. The viewport and the fractional scale are
    /// optional, together.
    pub(crate) fn usable(&self) -> bool {
        let compositor = self
            .compositor
            .is_some_and(|global| global.version >= OLDEST_COMPOSITOR);
        compositor && self.shm.is_some() && self.shell.is_some()
    }
}

/// What should show on every screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Wanted {
    /// A tray start's countdown in the middle: the label, in the job's
    /// colour.
    Countdown { label: Label, colour: Rgb },
    /// A run's badge, in the corner.
    Badge { badge: Badge, corner: Corner },
}

impl Wanted {
    /// What it shows, in a word: the label, or the badge's mark.
    pub(crate) fn text(&self) -> &str {
        match self {
            Wanted::Countdown { label, .. } => label.text(),
            Wanted::Badge { badge, .. } => match badge.mark {
                Mark::Pointer => "pointer",
                Mark::Dot => "dot",
                Mark::Triangle => "triangle",
            },
        }
    }

    /// Whether `other` is the same kind of show: another second of the
    /// countdown is, the badge after the countdown is not.
    pub(crate) fn same_kind(&self, other: &Wanted) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Its size and place on a screen `size` logical pixels large.
    fn fit(&self, size: (i32, i32)) -> Result<Fit, Missing> {
        match self {
            Wanted::Countdown { label, .. } => Fit::of(label, size),
            Wanted::Badge { corner, .. } => Fit::badge(*corner, size),
        }
    }
}

/// One screen as the display holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Showing {
    /// Its surface waits for the compositor's answer.
    Asked,
    /// Its surface shows this label, or this badge's mark.
    Drawn(String),
    /// It shows nothing, for this reason, until the screens change or the
    /// next show.
    Missing(Missing),
}

/// A surface to ask for: on the screen `name`, showing on `output`, `fit`
/// large.
#[derive(Clone, Copy)]
struct Asking<'a> {
    name: u32,
    output: &'a WlOutput,
    fit: Fit,
}

/// What the display's objects hold.
pub(crate) struct Surfaces {
    compositor: WlCompositor,
    shm: WlShm,
    shell: ZwlrLayerShellV1,
    /// Both or neither: a fractional scale is shown through a viewport.
    scaling: Option<(WpViewporter, WpFractionalScaleManagerV1)>,
    wanted: Option<Wanted>,
    /// By the output's global name.
    screens: BTreeMap<u32, Screen>,
    /// Each screen's logical size as last read: a screen whose size
    /// changed is tried again.
    sizes: BTreeMap<u32, (i32, i32)>,
}

enum Screen {
    /// Boxed: a surface's objects are many times a reason's size.
    Shown(Box<Shown>),
    Missing(Missing),
}

/// A screen's surface and what it shows.
struct Shown {
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    scale: Option<(WpViewport, WpFractionalScaleV1)>,
    /// Its size, as asked for: another size takes another surface.
    fit: Fit,
    /// The preferred scale in 120ths; [`WHOLE`] until the compositor says.
    preferred: u32,
    /// When the compositor's answer to the last size asked for is due;
    /// `None` once it came.
    answer: Option<Instant>,
    /// What was drawn last, at which scale and size.
    drawn: Option<(Wanted, u32, Fit)>,
    /// Its buffers the compositor has not released, at most [`LIVE`].
    buffers: Vec<WlBuffer>,
}

impl Shown {
    /// Destroys its objects, its buffers last: with the surface gone,
    /// nothing shows them, and the compositor frees their memory.
    fn destroy(self) {
        if let Some((viewport, fraction)) = self.scale {
            fraction.destroy();
            viewport.destroy();
        }
        self.layer.destroy();
        self.surface.destroy();
        for buffer in self.buffers {
            buffer.destroy();
        }
    }
}

impl Link {
    /// Binds the display's globals on this connection: `false` when it
    /// does not offer them.
    pub(crate) fn open_surfaces(&mut self) -> bool {
        let offered = self.client.display_globals;
        let (Some(compositor), Some(shm), Some(shell)) =
            (offered.compositor, offered.shm, offered.shell)
        else {
            return false;
        };
        let Some(registry) = self.client.registry.clone() else {
            return false;
        };
        if !offered.usable() {
            return false;
        }
        let queue = self.queue.handle();
        let scaling = offered
            .viewporter
            .zip(offered.fractions)
            .map(|(viewporter, fractions)| {
                (
                    bind(&registry, viewporter, VERSION_1, &queue),
                    bind(&registry, fractions, VERSION_1, &queue),
                )
            });
        self.client.surfaces = Some(Surfaces {
            compositor: bind(&registry, compositor, COMPOSITOR_VERSION, &queue),
            shm: bind(&registry, shm, VERSION_1, &queue),
            shell: bind(&registry, shell, SHELL_VERSION, &queue),
            scaling,
            wanted: None,
            screens: BTreeMap::new(),
            sizes: BTreeMap::new(),
        });
        true
    }

    /// Brings the screens in line with `wanted`, as of `now`: asks for a
    /// surface on each screen whose size is known and that has none, and
    /// draws at most one screen. Whether it drew one: another may be due.
    pub(crate) fn show(&mut self, wanted: Wanted, now: Instant) -> bool {
        let queue = self.queue.handle();
        let client = &mut self.client;
        let Some(surfaces) = &mut client.surfaces else {
            return false;
        };
        if surfaces
            .wanted
            .replace(wanted.clone())
            .is_some_and(|was| !was.same_kind(&wanted))
        {
            // The next show: a screen that showed nothing is tried again.
            surfaces.screens_changed();
        }
        let sizes = client.gather.sizes();
        surfaces.resized(&sizes);
        for (index, (&name, bound)) in client.outputs.iter().enumerate() {
            if index >= MAX_OUTPUTS {
                surfaces.miss(name, Missing::TooMany);
                continue;
            }
            let Some(&size) = sizes.get(&name) else {
                continue;
            };
            match wanted.fit(size) {
                Ok(fit) => {
                    let output = &bound.output;
                    surfaces.ask(Asking { name, output, fit }, &queue, now);
                }
                Err(missing) => surfaces.miss(name, missing),
            }
        }
        surfaces.draw_one(&queue)
    }

    /// Every surface whose answer is past due at `now` goes, its screen
    /// showing nothing: an answer after it shows nothing (O2).
    pub(crate) fn expire(&mut self, now: Instant) {
        if let Some(surfaces) = &mut self.client.surfaces {
            surfaces.expire(now);
        }
    }

    /// When the next answer is due, if one is awaited.
    pub(crate) fn next_answer(&self) -> Option<Instant> {
        let surfaces = self.client.surfaces.as_ref()?;
        let shown = surfaces.screens.values().filter_map(|screen| match screen {
            Screen::Shown(shown) => shown.answer,
            Screen::Missing(_) => None,
        });
        shown.min()
    }

    /// Each screen as the display holds it.
    pub(crate) fn showing(&self) -> BTreeMap<u32, Showing> {
        let Some(surfaces) = &self.client.surfaces else {
            return BTreeMap::new();
        };
        let showing = |screen: &Screen| match screen {
            Screen::Missing(missing) => Showing::Missing(*missing),
            Screen::Shown(shown) => match (&shown.drawn, shown.answer) {
                (Some((wanted, ..)), None) => Showing::Drawn(wanted.text().to_owned()),
                _ => Showing::Asked,
            },
        };
        let screens = surfaces.screens.iter();
        screens
            .map(|(&name, screen)| (name, showing(screen)))
            .collect()
    }
}

/// Places `layer` as `place` says: with no anchor, the compositor centres
/// it; in a corner, it is anchored to the corner's two edges, [`MARGIN`]
/// from each.
fn place(layer: &ZwlrLayerSurfaceV1, place: Place) {
    let Place::Corner(corner) = place else {
        return;
    };
    let (vertical, horizontal) = match corner {
        Corner::TopLeft => (Anchor::Top, Anchor::Left),
        Corner::TopRight => (Anchor::Top, Anchor::Right),
        Corner::BottomLeft => (Anchor::Bottom, Anchor::Left),
        Corner::BottomRight => (Anchor::Bottom, Anchor::Right),
    };
    layer.set_anchor(vertical | horizontal);
    // A margin on an edge it is not anchored to has no effect. 24: exact.
    let margin = MARGIN as i32;
    layer.set_margin(margin, margin, margin, margin);
}

fn bind<I: Proxy + 'static>(
    registry: &WlRegistry,
    global: Global,
    newest: u32,
    queue: &QueueHandle<Client>,
) -> I
where
    Client: wayland_client::Dispatch<I, ()>,
{
    registry.bind(global.name, global.version.min(newest), queue, ())
}

impl Surfaces {
    /// The screen `name` shows nothing, for `missing`: its surface goes.
    fn miss(&mut self, name: u32, missing: Missing) {
        let was = self.screens.insert(name, Screen::Missing(missing));
        match was {
            Some(Screen::Missing(before)) if before == missing => {}
            Some(Screen::Shown(shown)) => {
                shown.destroy();
                eprintln!(
                    "auto-clicker: the on-screen display shows on screen {name} no more: {missing}"
                );
            }
            _ => eprintln!(
                "auto-clicker: the on-screen display cannot show on screen {name}: {missing}"
            ),
        }
    }

    /// A screen whose size changed since it was last read shows nothing
    /// no more: it is tried again, as after a screen added or removed (O3).
    fn resized(&mut self, sizes: &BTreeMap<u32, (i32, i32)>) {
        for (name, size) in sizes {
            let was = self.sizes.insert(*name, *size);
            let missing = matches!(self.screens.get(name), Some(Screen::Missing(_)));
            if missing && was.is_some_and(|was| was != *size) {
                self.screens.remove(name);
            }
        }
    }

    /// Asks for the surface `asking` says, unless its screen has one that
    /// size or shows nothing. A surface of another size is made anew: an
    /// answer carries nothing to say which size it answers, so one to the
    /// old surface reaches nothing (O2).
    fn ask(&mut self, asking: Asking<'_>, queue: &QueueHandle<Client>, now: Instant) {
        let Asking { name, fit, .. } = asking;
        let due = now.checked_add(ANSWER).unwrap_or(now);
        match self.screens.get(&name) {
            Some(Screen::Missing(_)) => return,
            Some(Screen::Shown(shown)) if shown.fit == fit => return,
            Some(Screen::Shown(_)) | None => {}
        }
        if let Some(Screen::Shown(other)) = self.screens.remove(&name) {
            other.destroy();
        }
        let shown = self.create(asking, queue, due);
        self.screens.insert(name, Screen::Shown(Box::new(shown)));
    }

    /// The surface `asking` says, on the overlay layer in its place, taking
    /// no input; its answer due at `due`.
    fn create(&self, asking: Asking<'_>, queue: &QueueHandle<Client>, due: Instant) -> Shown {
        let Asking { name, output, fit } = asking;
        let surface = self.compositor.create_surface(queue, ());
        // Nothing of the surface takes input: clicks and the pointer go to
        // the game below.
        let nothing = self.compositor.create_region(queue, ());
        surface.set_input_region(Some(&nothing));
        nothing.destroy();
        let layer = self.shell.get_layer_surface(
            &surface,
            Some(output),
            Layer::Overlay,
            NAMESPACE.to_owned(),
            queue,
            name,
        );
        layer.set_size(fit.width, fit.height);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        place(&layer, fit.place);
        // Above everything, moving nothing, and placed on the screen's
        // edges whatever other surfaces keep for themselves.
        layer.set_exclusive_zone(-1);
        let scale = self.scaling.as_ref().map(|(viewporter, fractions)| {
            (
                viewporter.get_viewport(&surface, queue, ()),
                fractions.get_fractional_scale(&surface, queue, name),
            )
        });
        surface.commit();
        Shown {
            surface,
            layer,
            scale,
            fit,
            preferred: WHOLE,
            answer: Some(due),
            drawn: None,
            buffers: Vec::new(),
        }
    }

    /// Every surface whose answer is past due at `now` goes, its screen
    /// showing nothing: an answer after it shows nothing (O2).
    fn expire(&mut self, now: Instant) {
        let late: Vec<u32> = self
            .screens
            .iter()
            .filter_map(|(&name, screen)| match screen {
                Screen::Shown(shown) if shown.answer.is_some_and(|due| due <= now) => Some(name),
                _ => None,
            })
            .collect();
        for name in late {
            self.miss(name, Missing::Late);
        }
    }

    /// Draws the first screen answered whose drawing is not what should
    /// show; whether there was one.
    fn draw_one(&mut self, queue: &QueueHandle<Client>) -> bool {
        let Some(wanted) = self.wanted.clone() else {
            return false;
        };
        let due = self
            .screens
            .iter()
            .find_map(|(&name, screen)| match screen {
                Screen::Shown(shown) if shown.due(&wanted) => Some(name),
                _ => None,
            });
        let Some(name) = due else {
            return false;
        };
        if let Err(missing) = self.draw(name, &wanted, queue) {
            self.miss(name, missing);
        }
        true
    }

    /// Draws `wanted` on the screen `name` at its scale.
    fn draw(
        &mut self,
        name: u32,
        wanted: &Wanted,
        queue: &QueueHandle<Client>,
    ) -> Result<(), Missing> {
        let Some(Screen::Shown(shown)) = self.screens.get_mut(&name) else {
            return Ok(());
        };
        let scale = if shown.scale.is_some() {
            shown.preferred
        } else {
            WHOLE
        };
        let size = shown.fit.buffer(scale)?;
        let picture = match wanted {
            Wanted::Countdown { label, colour } => paint::number(label, *colour, size),
            Wanted::Badge { badge, .. } => paint::badge(*badge, size),
        };
        let buffer = shm::buffer(&self.shm, &picture, name, queue).map_err(|err| {
            eprintln!("auto-clicker: no memory for the on-screen display's buffer: {err}");
            Missing::Memory
        })?;
        let Fit { width, height, .. } = shown.fit;
        if let Some((viewport, _)) = &shown.scale {
            // Both at most MAX_SIDE.
            viewport.set_destination(width as i32, height as i32);
        }
        shown.surface.attach(Some(&buffer), 0, 0);
        shown
            .surface
            .damage_buffer(0, 0, size.width as i32, size.height as i32);
        shown.surface.commit();
        shown.drawn = Some((wanted.clone(), scale, shown.fit));
        shown.buffers.push(buffer);
        Ok(())
    }

    /// The screens came or went: each that showed nothing is tried again.
    fn screens_changed(&mut self) {
        self.screens
            .retain(|_, screen| !matches!(screen, Screen::Missing(_)));
    }

    /// The screen `name` went, and its surface with it.
    fn forget(&mut self, name: u32) {
        self.sizes.remove(&name);
        if let Some(Screen::Shown(shown)) = self.screens.remove(&name) {
            shown.destroy();
        }
    }
}

impl Shown {
    /// Whether it is to be drawn now: answered, not showing `wanted`, and
    /// with room for another buffer.
    fn due(&self, wanted: &Wanted) -> bool {
        self.answer.is_none() && !self.shows(wanted) && self.buffers.len() < LIVE
    }

    /// Whether it shows `wanted` at its scale and size now.
    fn shows(&self, wanted: &Wanted) -> bool {
        let scale = if self.scale.is_some() {
            self.preferred
        } else {
            WHOLE
        };
        self.drawn.as_ref() == Some(&(wanted.clone(), scale, self.fit))
    }
}

dispatch_to!(WlCompositor, (), ignore);
dispatch_to!(WlShm, (), ignore);
dispatch_to!(WlShmPool, (), ignore);
dispatch_to!(WlRegion, (), ignore);
dispatch_to!(WlSurface, (), ignore);
dispatch_to!(ZwlrLayerShellV1, (), ignore);
dispatch_to!(WpViewporter, (), ignore);
dispatch_to!(WpViewport, (), ignore);
dispatch_to!(WpFractionalScaleManagerV1, (), ignore);
dispatch_to!(WlBuffer, u32, buffer_event);
dispatch_to!(ZwlrLayerSurfaceV1, u32, layer_event);
dispatch_to!(WpFractionalScaleV1, u32, fraction_event);

impl Client {
    /// The outputs came or went: a screen that showed nothing is tried
    /// again (O3).
    pub(crate) fn surfaces_screens_changed(&mut self) {
        if let Some(surfaces) = &mut self.surfaces {
            surfaces.screens_changed();
        }
    }

    /// The global `name` went: an output's surface goes with it.
    pub(crate) fn surface_global_removed(&mut self, name: u32) {
        self.display_globals.removed(name);
        if let Some(surfaces) = &mut self.surfaces {
            surfaces.forget(name);
        }
    }

    /// A buffer the compositor is done with is destroyed: the memory goes
    /// with it, and its screen may draw again.
    fn buffer_event(&mut self, arrived: Arrived<'_, WlBuffer, u32>) {
        let wl_buffer::Event::Release = arrived.event else {
            return;
        };
        if let Some(surfaces) = &mut self.surfaces
            && let Some(Screen::Shown(shown)) = surfaces.screens.get_mut(arrived.data)
        {
            shown.buffers.retain(|buffer| buffer != arrived.proxy);
        }
        arrived.proxy.destroy();
    }

    /// The compositor's answer, which is acknowledged, or the surface
    /// closed, after which its screen shows nothing.
    fn layer_event(&mut self, arrived: Arrived<'_, ZwlrLayerSurfaceV1, u32>) {
        let Some(surfaces) = &mut self.surfaces else {
            return;
        };
        let name = *arrived.data;
        // Only the screen's live surface gets events: the library drops
        // those of an object destroyed, as a surface that went is.
        let Some(Screen::Shown(shown)) = surfaces.screens.get_mut(&name) else {
            return;
        };
        match arrived.event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                arrived.proxy.ack_configure(serial);
                shown.answer = None;
            }
            zwlr_layer_surface_v1::Event::Closed => surfaces.miss(name, Missing::Closed),
            _ => {}
        }
    }

    /// The screen's preferred scale: what its buffer is drawn at.
    fn fraction_event(&mut self, arrived: Arrived<'_, WpFractionalScaleV1, u32>) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = arrived.event else {
            return;
        };
        let Some(surfaces) = &mut self.surfaces else {
            return;
        };
        if let Some(Screen::Shown(shown)) = surfaces.screens.get_mut(arrived.data) {
            shown.preferred = scale;
        }
    }
}
