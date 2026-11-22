//! Cursor sessions on a recording's own connection (W02): the seat's
//! pointer, ext-image-copy-capture's two managers, and per output a source
//! and a pointer cursor session, whose positions [`Gather`] places on the
//! layout the connection settled.
//!
//! Nothing here asks for a frame: a cursor session is never a screen cast
//! (decision 0004, "niri cursor sessions"). The layout connection only
//! notes whether the globals are offered, and binds none of them (W7).
//!
//! [`Gather`]: crate::gather::Gather

use std::collections::BTreeMap;

use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::{Proxy, QueueHandle, WEnum};

use crate::NoScreens;
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_cursor_session_v1::{
    self, ExtImageCopyCaptureCursorSessionV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;

use super::{Arrived, Client, Global, Link, Mark};

pub(crate) const SEAT: &str = "wl_seat";
pub(crate) const CAPTURE_MANAGER: &str = "ext_image_copy_capture_manager_v1";
pub(crate) const SOURCE_MANAGER: &str = "ext_output_image_capture_source_manager_v1";

/// The newest versions the client knows: every request it makes is in the
/// first.
const SEAT_VERSION: u32 = 5;
const MANAGER_VERSION: u32 = 1;

/// The globals cursor sessions need, as offered.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Offered {
    seat: Option<Global>,
    capture: Option<Global>,
    sources: Option<Global>,
}

impl Offered {
    /// Notes the global `interface`, if it is one of these.
    pub(crate) fn note(&mut self, interface: &str, global: Global) {
        let slot = match interface {
            SEAT => &mut self.seat,
            CAPTURE_MANAGER => &mut self.capture,
            SOURCE_MANAGER => &mut self.sources,
            _ => return,
        };
        // The first seat offered: a desktop with one has no other.
        slot.get_or_insert(global);
    }

    /// The global `name` went.
    pub(crate) fn removed(&mut self, name: u32) {
        for slot in [&mut self.seat, &mut self.capture, &mut self.sources] {
            if slot.is_some_and(|global| global.name == name) {
                *slot = None;
            }
        }
    }

    /// Whether `name` is the seat offered.
    fn is_seat(&self, name: u32) -> bool {
        self.seat.is_some_and(|seat| seat.name == name)
    }

    /// Whether all three are offered.
    pub(crate) fn all(&self) -> bool {
        self.seat.is_some() && self.capture.is_some() && self.sources.is_some()
    }
}

/// Why a recording's cursor sessions could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The display no longer offers the globals.
    NotOffered,
    /// Its seat has no pointer.
    NoPointer,
}

/// What the cursor sessions hold.
pub(crate) struct Sessions {
    pointer: WlPointer,
    capture: ExtImageCopyCaptureManagerV1,
    sources: ExtOutputImageCaptureSourceManagerV1,
    /// By the output's global name.
    by_output: BTreeMap<u32, Session>,
}

/// One output's source and cursor session.
struct Session {
    source: ExtImageCaptureSourceV1,
    cursor: ExtImageCopyCaptureCursorSessionV1,
}

impl Sessions {
    /// A cursor session on `output`, the output `name`.
    fn open(&mut self, name: u32, output: &WlOutput, queue: &QueueHandle<Client>) {
        let source = self.sources.create_source(output, queue, ());
        let cursor =
            self.capture
                .create_pointer_cursor_session(&source, &self.pointer, queue, name);
        self.by_output.insert(name, Session { source, cursor });
    }

    /// Ends the output `name`'s session.
    fn close(&mut self, name: u32) {
        if let Some(Session { source, cursor }) = self.by_output.remove(&name) {
            cursor.destroy();
            source.destroy();
        }
    }
}

impl Link {
    /// Binds the seat, waits a round trip for what it can do, then binds
    /// the managers and opens a cursor session on each output bound. Called
    /// once the first layout settled, so the position each session sends
    /// at once is placed on it.
    pub(crate) async fn open_sessions(&mut self) -> Result<Result<(), Refused>, NoScreens> {
        let offered = self.client.cursor_globals;
        let (Some(seat), Some(capture), Some(sources)) =
            (offered.seat, offered.capture, offered.sources)
        else {
            return Ok(Err(Refused::NotOffered));
        };
        let queue = self.queue.handle();
        let Some(registry) = self.client.registry.clone() else {
            return Ok(Err(Refused::NotOffered));
        };
        let bound = seat.name;
        let seat: WlSeat = registry.bind(bound, seat.version.min(SEAT_VERSION), &queue, ());
        self.connection.display().sync(&queue, Mark::Seat);
        self.until(|client| client.seat_known).await?;
        if !self.client.cursor_globals.is_seat(bound) {
            return Ok(Err(Refused::NotOffered));
        }
        if !self.client.pointer_capable {
            return Ok(Err(Refused::NoPointer));
        }
        let sessions = Sessions {
            pointer: seat.get_pointer(&queue, ()),
            capture: bind(&registry, capture, &queue),
            sources: bind(&registry, sources, &queue),
            by_output: BTreeMap::new(),
        };
        self.client.sessions = Some(sessions);
        let outputs: Vec<(u32, WlOutput)> = self
            .client
            .outputs
            .iter()
            .map(|(&name, bound)| (name, bound.output.clone()))
            .collect();
        for (name, output) in outputs {
            self.client.open_session(name, &output, &queue);
        }
        self.flush().await?;
        Ok(Ok(()))
    }
}

fn bind<I: Proxy + 'static>(registry: &WlRegistry, global: Global, queue: &QueueHandle<Client>) -> I
where
    Client: wayland_client::Dispatch<I, ()>,
{
    registry.bind(global.name, global.version.min(MANAGER_VERSION), queue, ())
}

dispatch_to!(WlSeat, (), seat_event);
dispatch_to!(WlPointer, (), ignore);
dispatch_to!(ExtImageCopyCaptureManagerV1, (), ignore);
dispatch_to!(ExtOutputImageCaptureSourceManagerV1, (), ignore);
dispatch_to!(ExtImageCaptureSourceV1, (), ignore);
dispatch_to!(ExtImageCopyCaptureCursorSessionV1, u32, cursor_event);

impl Client {
    /// Opens a cursor session on the output `name` once sessions run: an
    /// output plugged in during a recording gets one too.
    pub(crate) fn open_session(
        &mut self,
        name: u32,
        output: &WlOutput,
        queue: &QueueHandle<Client>,
    ) {
        if let Some(sessions) = &mut self.sessions {
            sessions.open(name, output, queue);
        }
    }

    /// The global `name` went: an output's session ends with it, and the
    /// seat, bound or about to be, takes its pointer with it.
    pub(crate) fn cursor_global_removed(&mut self, name: u32) {
        if self.cursor_globals.is_seat(name) {
            self.pointer_capable = false;
        }
        self.cursor_globals.removed(name);
        if let Some(sessions) = &mut self.sessions {
            sessions.close(name);
        }
    }

    /// Whether the seat has a pointer: as it said last before the sessions
    /// opened, and since then whether it never lost it and was never
    /// removed, since the sessions stay on the pointer they were made with.
    pub(crate) fn has_pointer(&self) -> bool {
        self.pointer_capable
    }

    fn seat_event(&mut self, arrived: Arrived<'_, WlSeat, ()>) {
        if let wl_seat::Event::Capabilities { capabilities } = arrived.event {
            let has = matches!(
                capabilities,
                WEnum::Value(capabilities) if capabilities.contains(Capability::Pointer)
            );
            let kept = self.sessions.is_none() || self.pointer_capable;
            self.pointer_capable = has && kept;
        }
    }

    /// A position on the output whose name the session carries, placed
    /// when it is heard ([`Gather::position`]); entering, leaving and the
    /// hotspot say nothing a recording keeps.
    ///
    /// [`Gather::position`]: crate::gather::Gather::position
    fn cursor_event(&mut self, arrived: Arrived<'_, ExtImageCopyCaptureCursorSessionV1, u32>) {
        let ext_image_copy_capture_cursor_session_v1::Event::Position { x, y } = arrived.event
        else {
            return;
        };
        if let Some(placed) = self.gather.position(*arrived.data, x, y) {
            self.positions.push(placed);
        }
    }
}

#[cfg(test)]
mod tests;
