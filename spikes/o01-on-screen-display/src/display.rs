//! The display: one layer-shell surface per screen and picture, on the
//! overlay layer, asking for no keyboard and with an empty input region.
//! What shows is set by commands on stdin; screens added and removed while
//! it shows get it and lose it.

use std::time::Instant;

use wayland_client::globals::GlobalList;
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_output::WlOutput, wl_shm::WlShm, wl_surface::WlSurface,
};
use wayland_client::{Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1, zxdg_output_v1::ZxdgOutputV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use crate::paint::{self, Picture};
use crate::{say, shm};

/// The namespace every surface carries, for compositor rules.
pub const NAMESPACE: &str = "auto-clicker-on-screen";
/// The badge's distance from the screen's edges, in logical px.
const MARGIN: i32 = 24;

/// The globals the display uses.
pub struct Globals {
    pub compositor: WlCompositor,
    pub shm: WlShm,
    pub shell: ZwlrLayerShellV1,
    pub outputs: ZxdgOutputManagerV1,
    pub viewporter: Option<WpViewporter>,
    pub fractions: Option<WpFractionalScaleManagerV1>,
}

/// A screen, from its `wl_output` global.
pub struct Screen {
    pub global: u32,
    pub output: WlOutput,
    pub xdg: ZxdgOutputV1,
    pub name: String,
    /// Logical size, once xdg-output and `wl_output.done` gave it.
    pub logical: Option<(u32, u32)>,
    /// A logical size heard but not yet done.
    pub heard: Option<(u32, u32)>,
    /// The logical position, from xdg-output.
    pub position: (i32, i32),
}

/// A surface showing one picture on one screen.
pub struct Shown {
    pub id: u64,
    pub global: u32,
    pub picture: Picture,
    pub logical: (u32, u32),
    pub surface: WlSurface,
    pub layer: ZwlrLayerSurfaceV1,
    pub viewport: Option<WpViewport>,
    pub fraction: Option<WpFractionalScaleV1>,
    /// The preferred scale in 120ths; 120 until the compositor says.
    pub scale: u32,
    pub configured: bool,
}

/// One picture on one screen, by the screen's global.
#[derive(Clone, Copy)]
struct Slot {
    global: u32,
    picture: Picture,
}

pub struct State {
    pub globals: Globals,
    pub screens: Vec<Screen>,
    pub shown: Vec<Shown>,
    pub wanted: Vec<Picture>,
    next: u64,
}

impl State {
    pub fn new(globals: Globals) -> State {
        State {
            globals,
            screens: Vec::new(),
            shown: Vec::new(),
            wanted: Vec::new(),
            next: 0,
        }
    }

    /// Shows `wanted` on every screen whose size is known, and nothing else.
    pub fn set(&mut self, wanted: Vec<Picture>, qh: &QueueHandle<State>) {
        self.wanted = wanted;
        let globals: Vec<u32> = self.screens.iter().map(|screen| screen.global).collect();
        globals
            .into_iter()
            .for_each(|global| self.refresh(global, qh));
    }

    /// Brings one screen's surfaces in line with what is wanted.
    pub fn refresh(&mut self, global: u32, qh: &QueueHandle<State>) {
        let Some(logical) = self.screen(global).and_then(|screen| screen.logical) else {
            return;
        };
        let wanted = self.wanted.clone();
        let stale: Vec<u64> = self
            .shown
            .iter()
            .filter(|shown| {
                shown.global == global
                    && !wanted
                        .iter()
                        .any(|&picture| same_kind(picture, shown.picture))
            })
            .map(|shown| shown.id)
            .collect();
        stale
            .into_iter()
            .for_each(|id| self.remove(id, "not wanted"));
        for picture in wanted {
            self.place(Slot { global, picture }, logical, qh);
        }
    }

    /// Shows `slot`'s picture on a screen of logical size `screen`, reusing
    /// the surface already showing its kind there.
    fn place(&mut self, slot: Slot, screen: (u32, u32), qh: &QueueHandle<State>) {
        let Slot { global, picture } = slot;
        let logical = paint::logical_size(picture, screen);
        let existing = self
            .shown
            .iter()
            .position(|shown| shown.global == global && same_kind(shown.picture, picture));
        let Some(index) = existing else {
            return self.create(slot, logical, qh);
        };
        let shown = &mut self.shown[index];
        let changed = shown.picture != picture || shown.logical != logical;
        shown.picture = picture;
        if shown.logical != logical {
            shown.logical = logical;
            shown.layer.set_size(logical.0, logical.1);
            shown.surface.commit();
        } else if changed {
            self.draw(index, qh);
        }
    }

    fn create(&mut self, slot: Slot, logical: (u32, u32), qh: &QueueHandle<State>) {
        let Slot { global, picture } = slot;
        let Some(screen) = self.screen(global) else {
            return;
        };
        let output = screen.output.clone();
        let name = screen.name.clone();
        self.next += 1;
        let id = self.next;
        let globals = &self.globals;
        let surface = globals.compositor.create_surface(qh, id);
        // Nothing of the surface takes input: clicks fall through.
        let nothing = globals.compositor.create_region(qh, ());
        surface.set_input_region(Some(&nothing));
        nothing.destroy();
        let layer = globals.shell.get_layer_surface(
            &surface,
            Some(&output),
            Layer::Overlay,
            NAMESPACE.into(),
            qh,
            id,
        );
        layer.set_size(logical.0, logical.1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        if picture == Picture::Badge {
            layer.set_anchor(Anchor::Top | Anchor::Left);
            layer.set_margin(MARGIN, 0, 0, MARGIN);
        }
        let viewport = globals
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        let fraction = globals
            .fractions
            .as_ref()
            .map(|fractions| fractions.get_fractional_scale(&surface, qh, id));
        surface.commit();
        say!(
            "asked for {picture:?} on {name}: {}x{} logical, surface {}",
            logical.0,
            logical.1,
            surface.id()
        );
        self.shown.push(Shown {
            id,
            global,
            picture,
            logical,
            surface,
            layer,
            viewport,
            fraction,
            scale: 120,
            configured: false,
        });
    }

    /// Draws a configured surface's picture at its scale and commits it.
    pub fn draw(&mut self, index: usize, qh: &QueueHandle<State>) {
        let shown = &self.shown[index];
        if !shown.configured {
            return;
        }
        let (buffer_size, integer) = buffer_size(shown, self.globals.fractions.is_some());
        let started = Instant::now();
        let image = paint::draw(shown.picture, buffer_size);
        let painted = started.elapsed();
        let buffer = match shm::buffer(&self.globals.shm, &image, qh) {
            Ok(buffer) => buffer,
            Err(error) => return say!("no buffer for {:?}: {error}", shown.picture),
        };
        let shared = started.elapsed() - painted;
        match &shown.viewport {
            Some(viewport) => {
                viewport.set_destination(shown.logical.0 as i32, shown.logical.1 as i32)
            }
            None => shown.surface.set_buffer_scale(integer),
        }
        shown.surface.attach(Some(&buffer), 0, 0);
        shown
            .surface
            .damage_buffer(0, 0, image.width as i32, image.height as i32);
        shown.surface.commit();
        say!(
            "drew {:?} on {}: buffer {}x{} at scale {}/120 for {}x{} logical: paint {:.2} ms, shm {:.2} ms",
            shown.picture,
            self.screen_name(shown.global),
            image.width,
            image.height,
            shown.scale,
            shown.logical.0,
            shown.logical.1,
            painted.as_secs_f64() * 1e3,
            shared.as_secs_f64() * 1e3
        );
    }

    /// Destroys a surface and everything made for it.
    pub fn remove(&mut self, id: u64, why: &str) {
        let Some(index) = self.shown.iter().position(|shown| shown.id == id) else {
            return;
        };
        let shown = self.shown.remove(index);
        if let Some(fraction) = &shown.fraction {
            fraction.destroy();
        }
        if let Some(viewport) = &shown.viewport {
            viewport.destroy();
        }
        shown.layer.destroy();
        shown.surface.destroy();
        say!(
            "removed {:?} on {} ({why})",
            shown.picture,
            self.screen_name(shown.global)
        );
    }

    pub fn screen(&self, global: u32) -> Option<&Screen> {
        self.screens.iter().find(|screen| screen.global == global)
    }

    pub fn screen_name(&self, global: u32) -> String {
        self.screen(global)
            .map_or_else(|| format!("output {global}"), |screen| screen.name.clone())
    }

    pub fn index(&self, id: u64) -> Option<usize> {
        self.shown.iter().position(|shown| shown.id == id)
    }
}

fn same_kind(left: Picture, right: Picture) -> bool {
    std::mem::discriminant(&left) == std::mem::discriminant(&right)
}

/// The buffer's size: the logical size times the preferred fractional scale,
/// rounded, as wp-fractional-scale-v1 says; without it, the next whole scale.
fn buffer_size(shown: &Shown, fractional: bool) -> ((u32, u32), i32) {
    let scale = if fractional {
        shown.scale
    } else {
        shown.scale.div_ceil(120) * 120
    };
    let side = |logical: u32| (u64::from(logical) * u64::from(scale) + 60) / 120;
    let size = (side(shown.logical.0) as u32, side(shown.logical.1) as u32);
    (size, i32::try_from(scale / 120).unwrap_or(1).max(1))
}

/// Binds the display's globals; `None` names the first one missing.
pub fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Result<Globals, String> {
    let missing = |name: &'static str| move |_| format!("no {name}");
    Ok(Globals {
        compositor: globals
            .bind(qh, 4..=6, ())
            .map_err(missing("wl_compositor"))?,
        shm: globals.bind(qh, 1..=1, ()).map_err(missing("wl_shm"))?,
        shell: globals
            .bind(qh, 1..=4, ())
            .map_err(missing("zwlr_layer_shell_v1"))?,
        outputs: globals
            .bind(qh, 2..=3, ())
            .map_err(missing("zxdg_output_manager_v1"))?,
        viewporter: globals.bind(qh, 1..=1, ()).ok(),
        fractions: globals.bind(qh, 1..=1, ()).ok(),
    })
}
