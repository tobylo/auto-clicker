//! The test window: fullscreen, optionally holding a persistent pointer
//! lock, printing everything it hears. Motion is summed and printed at most
//! every 100 ms, so a 1 kHz mouse does not flood the pipe.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_seat::WlSeat, wl_shm::WlShm, wl_surface::WlSurface,
};
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_pointer_constraints_v1::{
    Lifetime, ZwpPointerConstraintsV1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1;
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;

use crate::dispatch::Heard;
use crate::paint::Image;
use crate::{dispatch, say, shm};

/// Summed motion of one kind.
#[derive(Default, Clone, Copy, PartialEq)]
pub struct Motion {
    pub count: u64,
    pub dx: f64,
    pub dy: f64,
}

pub struct State {
    pub shm: WlShm,
    pub surface: WlSurface,
    pub size: (i32, i32),
    pub relative: Motion,
    pub printed_relative: Motion,
    /// Absolute motion: count, and the last position in `dx`, `dy`.
    pub absolute: Motion,
    pub printed_absolute: Motion,
    pub closed: bool,
}

impl State {
    /// Fills the window with one dark colour and commits.
    pub fn paint(&self, qh: &QueueHandle<State>) {
        let (width, height) = (self.size.0.max(1) as u32, self.size.1.max(1) as u32);
        let pixel = 0xff20_2a33u32.to_le_bytes();
        let bytes = pixel.repeat(width as usize * height as usize);
        match shm::buffer(
            &self.shm,
            &Image {
                width,
                height,
                bytes,
            },
            qh,
        ) {
            Ok(buffer) => {
                self.surface.attach(Some(&buffer), 0, 0);
                self.surface
                    .damage_buffer(0, 0, width as i32, height as i32);
                self.surface.commit();
            }
            Err(error) => say!("no buffer: {error}"),
        }
    }

    /// Prints the motion summed since the last print, if any.
    pub fn report(&mut self) {
        if self.relative != self.printed_relative {
            let Motion { count, dx, dy } = self.relative;
            say!("relative total {count} sum {dx:.0},{dy:.0}");
            self.printed_relative = self.relative;
        }
        if self.absolute != self.printed_absolute {
            let Motion { count, dx, dy } = self.absolute;
            say!("absolute total {count} last {dx:.1},{dy:.1}");
            self.printed_absolute = self.absolute;
        }
    }
}

/// Opens the window; `lock` asks for a persistent pointer lock at once,
/// which the compositor activates once the window has the pointer.
pub fn open(
    globals: &GlobalList,
    qh: &QueueHandle<State>,
    lock: bool,
    output: Option<&WlOutput>,
) -> Result<State, String> {
    let missing = |name: &'static str| move |_| format!("no {name}");
    let compositor: WlCompositor = globals
        .bind(qh, 4..=6, ())
        .map_err(missing("wl_compositor"))?;
    let shm: WlShm = globals.bind(qh, 1..=1, ()).map_err(missing("wl_shm"))?;
    let base: XdgWmBase = globals
        .bind(qh, 1..=5, ())
        .map_err(missing("xdg_wm_base"))?;
    let seat: WlSeat = globals.bind(qh, 1..=7, ()).map_err(missing("wl_seat"))?;
    let surface = compositor.create_surface(qh, ());
    let xdg = base.get_xdg_surface(&surface, qh, ());
    let top = xdg.get_toplevel(qh, ());
    top.set_title("auto-clicker O01a test window".into());
    top.set_app_id("io.github.tobylo.AutoClickerO01aWindow".into());
    top.set_fullscreen(output);
    let pointer = seat.get_pointer(qh, ());
    seat.get_keyboard(qh, ());
    let relative: ZwpRelativePointerManagerV1 = globals
        .bind(qh, 1..=1, ())
        .map_err(missing("relative pointer"))?;
    relative.get_relative_pointer(&pointer, qh, ());
    if lock {
        let constraints: ZwpPointerConstraintsV1 = globals
            .bind(qh, 1..=1, ())
            .map_err(missing("pointer constraints"))?;
        constraints.lock_pointer(&surface, &pointer, None, Lifetime::Persistent, qh, ());
        say!(
            "asked for a persistent pointer lock on surface {}",
            surface.id()
        );
    }
    surface.commit();
    say!("window asked for, lock={lock}");
    Ok(State {
        shm,
        surface,
        size: (640, 480),
        relative: Motion::default(),
        printed_relative: Motion::default(),
        absolute: Motion::default(),
        printed_absolute: Motion::default(),
        closed: false,
    })
}

/// Output names heard, by global, on a queue of their own.
struct Names(Vec<(u32, String)>);

dispatch!(Names: WlOutput, u32 => heard_name);

fn heard_name(names: &mut Names, heard: Heard<WlOutput, u32, Names>) {
    if let wl_output::Event::Name { name } = heard.event {
        names.0.push((*heard.data, name));
    }
}

/// The output called `name`, found on a queue of its own before the window
/// exists.
pub fn output(
    connection: &Connection,
    globals: &GlobalList,
    name: &str,
) -> Result<WlOutput, String> {
    let mut queue = connection.new_event_queue::<Names>();
    let qh = queue.handle();
    let outputs: Vec<(u32, WlOutput)> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|global| global.interface == "wl_output")
        .map(|global| {
            (
                global.name,
                globals
                    .registry()
                    .bind(global.name, global.version.min(4), &qh, global.name),
            )
        })
        .collect();
    let mut names = Names(Vec::new());
    queue
        .roundtrip(&mut names)
        .map_err(|error| format!("{error}"))?;
    outputs
        .into_iter()
        .find(|(global, _)| {
            names
                .0
                .iter()
                .any(|(heard, text)| heard == global && text == name)
        })
        .map(|(_, output)| output)
        .ok_or_else(|| format!("no output called {name}"))
}
