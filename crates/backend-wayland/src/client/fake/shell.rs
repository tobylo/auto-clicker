//! The fake compositor's side of the on-screen display: the globals it
//! offers, and every surface, layer surface, region, viewport, fractional
//! scale, pool and buffer the client makes, as read from its requests.

use std::collections::BTreeMap;

use super::{DISPLAY, Ids, Message, Name, string};

pub(crate) const COMPOSITOR: Name = Name(5);
pub(crate) const SHM: Name = Name(6);
pub(crate) const SHELL: Name = Name(7);
pub(crate) const VIEWPORTER: Name = Name(8);
pub(crate) const FRACTIONS: Name = Name(9);

/// The overlay layer's value.
pub(crate) const OVERLAY: u32 = 3;

/// One `wl_surface` and what was asked of it and of its layer surface.
#[derive(Clone, Debug, Default)]
pub(crate) struct Surface {
    pub(crate) id: u32,
    pub(crate) layer: Option<u32>,
    /// The output it was asked on, as its `wl_output` id.
    pub(crate) output: Option<u32>,
    pub(crate) layer_kind: Option<u32>,
    pub(crate) namespace: Option<String>,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) keyboard: Option<u32>,
    pub(crate) exclusive: Option<i32>,
    pub(crate) anchor: Option<u32>,
    /// The region set as its input region; `Some(0)` for none set at all.
    pub(crate) input: Option<u32>,
    pub(crate) viewport: Option<u32>,
    pub(crate) fraction: Option<u32>,
    pub(crate) destination: Option<(i32, i32)>,
    /// Each buffer attached, by id.
    pub(crate) attached: Vec<u32>,
    pub(crate) acked: Vec<u32>,
    pub(crate) commits: usize,
    pub(crate) destroyed: bool,
    pub(crate) layer_destroyed: bool,
}

/// A buffer: its size and stride, and its format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Buffer {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: u32,
    pub(crate) format: u32,
}

/// What an object the client made is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Surface,
    Region,
    Layer,
    Pool,
    Buffer,
    Viewport,
    Fraction,
}

/// Everything of the display the fake has read.
#[derive(Debug, Default)]
pub(crate) struct Shell {
    pub(crate) surfaces: Vec<Surface>,
    /// Each region's rectangles added.
    pub(crate) regions: BTreeMap<u32, Vec<[u32; 4]>>,
    pub(crate) buffers: BTreeMap<u32, Buffer>,
    /// Each buffer destroyed, by id.
    pub(crate) destroyed: Vec<u32>,
    kinds: BTreeMap<u32, Kind>,
}

impl Shell {
    /// Notes `request`, made on a global bound as `global` if it was one.
    pub(crate) fn note(&mut self, request: &Message, global: Option<Name>) {
        if let Some(global) = global {
            self.note_global(global, request);
            return;
        }
        match self.kinds.get(&request.object).copied() {
            Some(Kind::Surface) => self.note_surface(request),
            Some(Kind::Layer) => self.note_layer(request),
            Some(Kind::Region) => self.note_region(request),
            Some(Kind::Pool) => self.note_pool(request),
            Some(Kind::Viewport) => self.note_viewport(request),
            Some(Kind::Buffer) if request.opcode == 0 => self.destroyed.push(request.object),
            _ => {}
        }
    }

    /// `wl_region.add`: a rectangle the region takes in.
    fn note_region(&mut self, request: &Message) {
        let words = &request.words;
        if request.opcode == 1 {
            let rect = [words[0], words[1], words[2], words[3]];
            self.regions.entry(request.object).or_default().push(rect);
        }
    }

    /// `wl_shm_pool.create_buffer`: new id, offset, width, height, stride
    /// and format.
    fn note_pool(&mut self, request: &Message) {
        let words = &request.words;
        if request.opcode != 0 {
            return;
        }
        let buffer = Buffer {
            width: words[2],
            height: words[3],
            stride: words[4],
            format: words[5],
        };
        self.kinds.insert(words[0], Kind::Buffer);
        self.buffers.insert(words[0], buffer);
    }

    /// `wp_viewport.set_destination`: the size the buffer shows at.
    fn note_viewport(&mut self, request: &Message) {
        let (object, words) = (request.object, &request.words);
        if request.opcode != 2 {
            return;
        }
        let destination = (words[0].cast_signed(), words[1].cast_signed());
        if let Some(surface) = self.surface_by(|s| s.viewport == Some(object)) {
            surface.destination = Some(destination);
        }
    }

    fn note_global(&mut self, global: Name, request: &Message) {
        let words = &request.words;
        let made = words.first().copied().unwrap_or_default();
        let kind = match (global, request.opcode) {
            (COMPOSITOR, 0) => {
                self.surfaces.push(Surface {
                    id: made,
                    ..Surface::default()
                });
                Kind::Surface
            }
            (COMPOSITOR, 1) => {
                self.regions.insert(made, Vec::new());
                Kind::Region
            }
            (SHM, 0) => Kind::Pool,
            (SHELL, 0) => self.layer_made(request),
            (VIEWPORTER, 1) => {
                self.on_surface(words[1], |surface| surface.viewport = Some(made));
                Kind::Viewport
            }
            (FRACTIONS, 1) => {
                self.on_surface(words[1], |surface| surface.fraction = Some(made));
                Kind::Fraction
            }
            _ => return,
        };
        self.kinds.insert(made, kind);
    }

    /// `zwlr_layer_shell_v1.get_layer_surface`: new id, surface, output,
    /// layer and namespace.
    fn layer_made(&mut self, request: &Message) -> Kind {
        let words = &request.words;
        let namespace = namespace(request);
        self.on_surface(words[1], |made| {
            made.layer = Some(words[0]);
            made.output = Some(words[2]);
            made.layer_kind = Some(words[3]);
            made.namespace = Some(namespace);
        });
        Kind::Layer
    }

    /// Changes the surface whose id is `id`, if there is one.
    fn on_surface(&mut self, id: u32, change: impl FnOnce(&mut Surface)) {
        if let Some(surface) = self.surface_by(|s| s.id == id) {
            change(surface);
        }
    }

    fn note_surface(&mut self, request: &Message) {
        let words = &request.words;
        let object = request.object;
        let Some(surface) = self.surface_by(|s| s.id == object) else {
            return;
        };
        match request.opcode {
            0 => surface.destroyed = true,
            1 => surface.attached.push(words[0]),
            5 => surface.input = Some(words[0]),
            6 => surface.commits += 1,
            _ => {}
        }
    }

    fn note_layer(&mut self, request: &Message) {
        let words = &request.words;
        let object = request.object;
        let Some(surface) = self.surface_by(|s| s.layer == Some(object)) else {
            return;
        };
        match request.opcode {
            0 => surface.size = Some((words[0], words[1])),
            1 => surface.anchor = Some(words[0]),
            2 => surface.exclusive = Some(words[0].cast_signed()),
            4 => surface.keyboard = Some(words[0]),
            6 => surface.acked.push(words[0]),
            7 => surface.layer_destroyed = true,
            _ => {}
        }
    }

    fn surface_by(&mut self, found: impl Fn(&Surface) -> bool) -> Option<&mut Surface> {
        self.surfaces
            .iter_mut()
            .rev()
            .find(|surface| found(surface))
    }

    /// The live layer surface on the output `ids` names.
    pub(crate) fn on(&self, ids: Ids) -> Option<&Surface> {
        let live = |surface: &&Surface| !surface.destroyed && surface.layer.is_some();
        self.surfaces
            .iter()
            .rev()
            .filter(live)
            .find(|surface| surface.output == Some(ids.output))
    }
}

/// `get_layer_surface`'s namespace, the string from its fifth word on.
fn namespace(request: &Message) -> String {
    let words = &request.words[4..];
    let length = usize::try_from(words[0]).unwrap();
    let bytes: Vec<u8> = words[1..]
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .take(length.saturating_sub(1))
        .collect();
    String::from_utf8(bytes).unwrap()
}

impl Surface {
    /// `zwlr_layer_surface_v1.configure` on its layer surface at `serial`,
    /// the size it asked for.
    pub(crate) fn configure(&self, serial: u32) -> Message {
        let (width, height) = self.size.unwrap();
        Message {
            object: self.layer.unwrap(),
            opcode: 0,
            words: vec![serial, width, height],
        }
    }

    /// `zwlr_layer_surface_v1.closed` on its layer surface.
    pub(crate) fn closed(&self) -> Message {
        Message {
            object: self.layer.unwrap(),
            opcode: 1,
            words: Vec::new(),
        }
    }

    /// `wp_fractional_scale_v1.preferred_scale` on its fractional scale,
    /// in 120ths.
    pub(crate) fn preferred(&self, scale: u32) -> Message {
        Message {
            object: self.fraction.unwrap(),
            opcode: 0,
            words: vec![scale],
        }
    }

    /// `wl_display.error` on its layer surface: the compositor refuses the
    /// connection.
    pub(crate) fn error(&self) -> Message {
        let object = self.layer.unwrap();
        Message {
            object: DISPLAY,
            opcode: 0,
            words: [object, 1].into_iter().chain(string("refused")).collect(),
        }
    }
}

/// `wl_buffer.release` on `buffer`: the compositor is done with it.
pub(crate) fn release(buffer: u32) -> Message {
    Message {
        object: buffer,
        opcode: 0,
        words: Vec::new(),
    }
}
