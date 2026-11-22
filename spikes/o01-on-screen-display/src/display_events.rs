//! What the compositor tells the display, logged as it comes.

use wayland_client::globals::{Global, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_output::{self, WlOutput},
    wl_region::WlRegion,
    wl_registry::{self, WlRegistry},
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_surface::{self, WlSurface},
};
use wayland_client::{Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

use crate::dispatch::Heard;
use crate::display::{Screen, State};
use crate::{dispatch, say};

delegate_noop!(State: WlCompositor);
delegate_noop!(State: WlRegion);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: ZxdgOutputManagerV1);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
delegate_noop!(State: WpFractionalScaleManagerV1);

/// Adds a screen for a `wl_output` global.
pub fn add_screen(
    state: &mut State,
    registry: &WlRegistry,
    heard: &Global,
    qh: &QueueHandle<State>,
) {
    let global = heard.name;
    let output: WlOutput = registry.bind(global, heard.version.min(4), qh, global);
    let xdg = state.globals.outputs.get_xdg_output(&output, qh, global);
    let name = format!("output {global}");
    state.screens.push(Screen {
        global,
        output,
        xdg,
        name,
        logical: None,
        heard: None,
        position: (0, 0),
    });
    say!("output global {global} added");
}

fn remove_screen(state: &mut State, global: u32) {
    let ids: Vec<u64> = state
        .shown
        .iter()
        .filter(|shown| shown.global == global)
        .map(|shown| shown.id)
        .collect();
    ids.into_iter()
        .for_each(|id| state.remove(id, "its output went"));
    let Some(index) = state
        .screens
        .iter()
        .position(|screen| screen.global == global)
    else {
        return;
    };
    let screen = state.screens.remove(index);
    screen.xdg.destroy();
    if screen.output.version() >= 3 {
        screen.output.release();
    }
    say!("output {} (global {global}) removed", screen.name);
}

dispatch!(State: WlRegistry, GlobalListContents => registry);
dispatch!(State: WlOutput, u32 => output);
dispatch!(State: ZxdgOutputV1, u32 => xdg_output);
dispatch!(State: ZwlrLayerSurfaceV1, u64 => layer_surface);
dispatch!(State: WpFractionalScaleV1, u64 => fractional_scale);
dispatch!(State: WlSurface, u64 => surface);
dispatch!(State: WlBuffer, () => buffer);

fn registry(state: &mut State, heard: Heard<WlRegistry, GlobalListContents, State>) {
    match heard.event {
        wl_registry::Event::Global {
            name,
            interface,
            version,
        } if interface == "wl_output" => {
            add_screen(
                state,
                heard.proxy,
                &Global {
                    name,
                    interface,
                    version,
                },
                heard.qh,
            );
        }
        wl_registry::Event::GlobalRemove { name } if state.screen(name).is_some() => {
            remove_screen(state, name)
        }
        _ => {}
    }
}

fn output(state: &mut State, heard: Heard<WlOutput, u32, State>) {
    let global = *heard.data;
    let Some(screen) = state
        .screens
        .iter_mut()
        .find(|screen| screen.global == global)
    else {
        return;
    };
    match heard.event {
        wl_output::Event::Name { name } => screen.name = name,
        wl_output::Event::Done => output_done(state, global, heard.qh),
        _ => {}
    }
}

/// Takes a logical size heard since the last `done`, if it changed.
fn output_done(state: &mut State, global: u32, qh: &QueueHandle<State>) {
    let Some(screen) = state
        .screens
        .iter_mut()
        .find(|screen| screen.global == global)
    else {
        return;
    };
    if screen.heard.is_none() || screen.heard == screen.logical {
        return;
    }
    screen.logical = screen.heard;
    let ((width, height), (x, y)) = (screen.logical.unwrap_or_default(), screen.position);
    say!(
        "output {} is {width}x{height} at {x},{y} logical",
        screen.name
    );
    state.refresh(global, qh);
}

fn xdg_output(state: &mut State, heard: Heard<ZxdgOutputV1, u32, State>) {
    let global = *heard.data;
    let Some(screen) = state
        .screens
        .iter_mut()
        .find(|screen| screen.global == global)
    else {
        return;
    };
    match heard.event {
        zxdg_output_v1::Event::LogicalSize { width, height } => {
            screen.heard = Some((width.max(0) as u32, height.max(0) as u32));
        }
        zxdg_output_v1::Event::LogicalPosition { x, y } => screen.position = (x, y),
        _ => {}
    }
}

fn layer_surface(state: &mut State, heard: Heard<ZwlrLayerSurfaceV1, u64, State>) {
    let id = *heard.data;
    let Some(index) = state.index(id) else { return };
    match heard.event {
        zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } => {
            heard.proxy.ack_configure(serial);
            state.shown[index].configured = true;
            let (picture, global) = (state.shown[index].picture, state.shown[index].global);
            say!(
                "configured {picture:?} at {width}x{height} on {}",
                state.screen_name(global)
            );
            state.draw(index, heard.qh);
        }
        zwlr_layer_surface_v1::Event::Closed => state.remove(id, "closed by the compositor"),
        _ => {}
    }
}

fn fractional_scale(state: &mut State, heard: Heard<WpFractionalScaleV1, u64, State>) {
    let Some(index) = state.index(*heard.data) else {
        return;
    };
    let wp_fractional_scale_v1::Event::PreferredScale { scale } = heard.event else {
        return;
    };
    say!(
        "preferred scale {scale}/120 for {:?}",
        state.shown[index].picture
    );
    if state.shown[index].scale != scale {
        state.shown[index].scale = scale;
        state.draw(index, heard.qh);
    }
}

fn surface(state: &mut State, heard: Heard<WlSurface, u64, State>) {
    let Some(index) = state.index(*heard.data) else {
        return;
    };
    let picture = state.shown[index].picture;
    match heard.event {
        wl_surface::Event::Enter { output } => {
            let global = output.data::<u32>().copied().unwrap_or(0);
            say!("{picture:?} entered {}", state.screen_name(global));
        }
        wl_surface::Event::Leave { .. } => say!("{picture:?} left an output"),
        wl_surface::Event::PreferredBufferScale { factor } => {
            say!("{picture:?}: preferred buffer scale {factor}")
        }
        _ => {}
    }
}

fn buffer(_: &mut State, heard: Heard<WlBuffer, (), State>) {
    // Each draw makes a new buffer; the old one goes once released.
    if let wl_buffer::Event::Release = heard.event {
        heard.proxy.destroy();
    }
}
