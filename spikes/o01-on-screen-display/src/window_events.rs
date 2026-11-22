//! What the test window hears, logged as it comes.

use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_keyboard::{self, WlKeyboard},
    wl_pointer::{self, WlPointer},
    wl_registry::WlRegistry,
    wl_seat::WlSeat,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{WEnum, delegate_noop};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_locked_pointer_v1::{self, ZwpLockedPointerV1},
    zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
    zwp_relative_pointer_v1::{self, ZwpRelativePointerV1},
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

use crate::dispatch::Heard;
use crate::window::State;
use crate::{dispatch, say};

delegate_noop!(State: WlCompositor);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ZwpPointerConstraintsV1);
delegate_noop!(State: ZwpRelativePointerManagerV1);

dispatch!(State: WlRegistry, GlobalListContents => registry);
dispatch!(State: WlBuffer, () => buffer);
dispatch!(State: XdgWmBase, () => wm_base);
dispatch!(State: XdgSurface, () => xdg_surface);
dispatch!(State: XdgToplevel, () => toplevel);
dispatch!(State: WlKeyboard, () => keyboard);
dispatch!(State: WlPointer, () => pointer);
dispatch!(State: ZwpLockedPointerV1, () => locked_pointer);
dispatch!(State: ZwpRelativePointerV1, () => relative_pointer);

fn registry(_: &mut State, _: Heard<WlRegistry, GlobalListContents, State>) {}

fn buffer(_: &mut State, heard: Heard<WlBuffer, (), State>) {
    if let wl_buffer::Event::Release = heard.event {
        heard.proxy.destroy();
    }
}

fn wm_base(_: &mut State, heard: Heard<XdgWmBase, (), State>) {
    if let xdg_wm_base::Event::Ping { serial } = heard.event {
        heard.proxy.pong(serial);
    }
}

fn xdg_surface(state: &mut State, heard: Heard<XdgSurface, (), State>) {
    if let xdg_surface::Event::Configure { serial } = heard.event {
        heard.proxy.ack_configure(serial);
        state.paint(heard.qh);
    }
}

fn has(states: &[u8], wanted: xdg_toplevel::State) -> bool {
    let (words, _) = states.as_chunks::<4>();
    words
        .iter()
        .any(|&word| u32::from_ne_bytes(word) == wanted as u32)
}

fn toplevel(state: &mut State, heard: Heard<XdgToplevel, (), State>) {
    match heard.event {
        xdg_toplevel::Event::Configure {
            width,
            height,
            states,
        } => {
            let fullscreen = has(&states, xdg_toplevel::State::Fullscreen);
            let activated = has(&states, xdg_toplevel::State::Activated);
            say!(
                "window configured {width}x{height} fullscreen={fullscreen} activated={activated}"
            );
            if width > 0 && height > 0 {
                state.size = (width, height);
            }
        }
        xdg_toplevel::Event::Close => state.closed = true,
        _ => {}
    }
}

fn keyboard(state: &mut State, heard: Heard<WlKeyboard, (), State>) {
    match heard.event {
        wl_keyboard::Event::Enter { .. } => say!("keyboard focus: gained"),
        wl_keyboard::Event::Leave { .. } => {
            state.report();
            say!("keyboard focus: LOST");
        }
        wl_keyboard::Event::Key {
            key,
            state: pressed,
            ..
        } => say!("key {key} {pressed:?}"),
        _ => {}
    }
}

fn pointer(state: &mut State, heard: Heard<WlPointer, (), State>) {
    match heard.event {
        wl_pointer::Event::Enter {
            surface_x,
            surface_y,
            ..
        } => {
            (state.absolute.dx, state.absolute.dy) = (surface_x, surface_y);
            state.printed_absolute = state.absolute;
            say!("pointer focus: gained at {surface_x:.1},{surface_y:.1}");
        }
        wl_pointer::Event::Leave { .. } => {
            state.report();
            say!("pointer focus: LOST");
        }
        wl_pointer::Event::Motion {
            surface_x,
            surface_y,
            ..
        } => {
            state.absolute.count += 1;
            (state.absolute.dx, state.absolute.dy) = (surface_x, surface_y);
        }
        wl_pointer::Event::Button {
            button,
            state: pressed,
            ..
        } => button_event(state, button, pressed),
        _ => {}
    }
}

fn button_event(state: &mut State, button: u32, pressed: WEnum<wl_pointer::ButtonState>) {
    state.report();
    let pressed = matches!(pressed, WEnum::Value(wl_pointer::ButtonState::Pressed));
    let what = if pressed { "press" } else { "release" };
    say!(
        "button {button:#x} {what} at {:.1},{:.1}",
        state.absolute.dx,
        state.absolute.dy
    );
}

fn locked_pointer(state: &mut State, heard: Heard<ZwpLockedPointerV1, (), State>) {
    state.report();
    match heard.event {
        zwp_locked_pointer_v1::Event::Locked => say!("pointer lock: locked"),
        zwp_locked_pointer_v1::Event::Unlocked => say!("pointer lock: UNLOCKED"),
        _ => {}
    }
}

fn relative_pointer(state: &mut State, heard: Heard<ZwpRelativePointerV1, (), State>) {
    if let zwp_relative_pointer_v1::Event::RelativeMotion {
        dx_unaccel,
        dy_unaccel,
        ..
    } = heard.event
    {
        state.relative.count += 1;
        state.relative.dx += dx_unaccel;
        state.relative.dy += dy_unaccel;
    }
}
