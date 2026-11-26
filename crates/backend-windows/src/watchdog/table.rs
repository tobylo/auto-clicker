//! The table the daemon and its watchdog share: one byte for every Linux
//! key code up to `KEY_MAX`, the mouse buttons included at their own codes
//! (`BTN_LEFT` is 0x110). A byte that is not 0 says a release is owed for
//! that code. Indexed by code, the table is never full.
//!
//! The daemon marks a code before it sends the press and clears it only
//! once the release is delivered ([`crate::watched`]). The watchdog reads
//! the table once, after the daemon ended, and trusts nothing in it: it
//! copies the bytes and keeps only releases of codes that are a key or a
//! button ([`Owed`]).

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{NonNull, null};
use std::sync::atomic::{AtomicU8, Ordering};

use auto_clicker_core::input::{Button, Held, Key};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};

/// Linux's `KEY_MAX`: the highest key code.
const KEY_MAX: usize = 0x2ff;
/// The table's length in bytes: one for each code from 0 to `KEY_MAX`.
pub const TABLE_LEN: usize = KEY_MAX + 1;

/// The table's byte for a code that owes nothing.
const FREE: u8 = 0;
/// The byte the daemon writes for a code that owes a release.
const OWED: u8 = 1;

/// The shared table, as one process maps it.
pub struct Table {
    cells: NonNull<AtomicU8>,
    /// What keeps `cells` alive.
    _keep: Keep,
}

// SAFETY: the cells are atomics, which any thread may read and write; the
// mapping or the buffer they live in is freed only when the table is
// dropped, which needs it owned.
#[allow(unsafe_code)]
unsafe impl Send for Table {}
// SAFETY: as above; every access through `&Table` is atomic.
#[allow(unsafe_code)]
unsafe impl Sync for Table {}

/// What a table's cells live in.
enum Keep {
    /// A view of the shared section, unmapped when dropped.
    View { _view: View },
    /// Memory of this process alone, for tests.
    #[cfg(test)]
    Own { _cells: Box<[AtomicU8]> },
}

impl Table {
    /// Whether a release is owed for `item`.
    pub fn owes(&self, item: Held) -> bool {
        self.cell(item).load(Ordering::Acquire) != FREE
    }

    /// Marks `item` as owing a release: done before its press is sent.
    pub fn mark(&self, item: Held) {
        self.cell(item).store(OWED, Ordering::Release);
    }

    /// Clears `item`: done once its release is delivered.
    pub fn clear(&self, item: Held) {
        self.cell(item).store(FREE, Ordering::Release);
    }

    /// A copy of every byte, read once: what the watchdog parses.
    pub fn copy(&self) -> [u8; TABLE_LEN] {
        let mut bytes = [FREE; TABLE_LEN];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.at(index).load(Ordering::Acquire);
        }
        bytes
    }

    /// A table of this process alone, every code free.
    #[cfg(test)]
    pub(crate) fn private() -> Table {
        let cells: Box<[AtomicU8]> = (0..TABLE_LEN).map(|_| AtomicU8::new(FREE)).collect();
        let first = NonNull::from(&cells[0]);
        Table {
            cells: first,
            _keep: Keep::Own { _cells: cells },
        }
    }

    fn cell(&self, item: Held) -> &AtomicU8 {
        self.at(usize::from(code(item)))
    }

    /// The cell at `index`, which must be below [`TABLE_LEN`]: every code
    /// [`code`] gives is, and [`Table::copy`] counts up to it.
    #[allow(unsafe_code)]
    fn at(&self, index: usize) -> &AtomicU8 {
        assert!(index < TABLE_LEN, "table index {index} past its end");
        // SAFETY: `cells` points at `TABLE_LEN` initialised atomics that
        // live as long as `self` (the view or the buffer in `_keep`), and
        // `index` is below that, checked above.
        unsafe { &*self.cells.as_ptr().add(index) }
    }
}

/// The table's index for `item`: its Linux code. Keys run from 1 to 248 and
/// buttons from 0x110 to 0x114, all below [`TABLE_LEN`].
fn code(item: Held) -> u16 {
    match item {
        Held::Key(key) => key.code(),
        Held::Button(button) => button.code(),
    }
}

/// The releases a dead daemon's table owes, parsed once from a copy of its
/// bytes. Holds only items that are a key or a button, so it can only ever
/// be released.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Owed {
    items: Vec<Held>,
    /// How many non-zero bytes named no key or button.
    pub rejected: usize,
}

impl Owed {
    /// What `bytes` owes. Any byte that is not 0 owes a release, whatever
    /// its value; one at a code that is neither a key nor a button is
    /// counted in [`Owed::rejected`] and never sent.
    pub fn parse(bytes: &[u8; TABLE_LEN]) -> Owed {
        let mut owed = Owed::default();
        for (index, &byte) in bytes.iter().enumerate() {
            if byte == FREE {
                continue;
            }
            match u16::try_from(index).ok().and_then(item) {
                Some(found) => owed.items.push(found),
                None => owed.rejected = owed.rejected.saturating_add(1),
            }
        }
        owed
    }

    /// The items to release, in code order, each once.
    pub fn items(&self) -> &[Held] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The key or button with Linux code `code`, if there is one.
fn item(code: u16) -> Option<Held> {
    Button::from_code(code)
        .map(Held::Button)
        .or_else(|| Key::new(code).map(Held::Key))
}

/// The unnamed section the table lives in, which the watchdog creates and
/// its daemon inherits.
pub struct Section {
    handle: OwnedHandle,
    table: Table,
}

impl Section {
    /// Creates the section with an inheritable handle, zeroed, and maps it.
    /// Only a child started with this handle in its list inherits it
    /// ([`crate::watchdog::start_daemon`]).
    ///
    /// # Errors
    /// Windows could not create or map it.
    #[allow(unsafe_code)]
    pub fn create() -> io::Result<Section> {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let size = u32::try_from(TABLE_LEN).unwrap_or(u32::MAX);
        // SAFETY: backed by the paging file (no file handle), the attributes
        // live for the call, no name. Windows zeroes a new section.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                &attributes,
                PAGE_READWRITE,
                0,
                size,
                null(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let table = map(handle.as_raw_handle())?;
        Ok(Section { handle, table })
    }

    /// The table, as this process maps it.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The section's handle, inherited by the daemon.
    pub(crate) fn raw(&self) -> HANDLE {
        self.handle.as_raw_handle()
    }
}

/// Maps `TABLE_LEN` bytes of the section `handle` for reading and writing.
/// A handle that is no section, or a section smaller than the table, fails.
#[allow(unsafe_code)]
pub(crate) fn map(handle: HANDLE) -> io::Result<Table> {
    // SAFETY: `handle` is passed to Windows as it is: anything but a
    // section of at least `TABLE_LEN` bytes fails, and nothing is read
    // from it here.
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, TABLE_LEN) };
    let Some(cells) = NonNull::new(view.Value.cast::<AtomicU8>()) else {
        return Err(io::Error::last_os_error());
    };
    Ok(Table {
        cells,
        _keep: Keep::View { _view: View(view) },
    })
}

/// A mapped view, unmapped when dropped.
struct View(MEMORY_MAPPED_VIEW_ADDRESS);

impl Drop for View {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the view came from `MapViewOfFile` and is unmapped once;
        // the table that pointed into it is being dropped.
        unsafe { UnmapViewOfFile(self.0) };
    }
}

#[cfg(test)]
mod tests;
