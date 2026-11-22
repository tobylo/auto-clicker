//! A buffer in shared memory from an anonymous file: `memfd_create` through
//! rustix, written with `pwrite`, so neither a named file under
//! `XDG_RUNTIME_DIR` nor `mmap` (and its `unsafe`) is needed.

use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use wayland_client::protocol::{wl_buffer::WlBuffer, wl_shm, wl_shm_pool::WlShmPool};
use wayland_client::{Dispatch, QueueHandle};

use crate::paint::Image;

/// Why a buffer could not be made.
#[derive(Debug)]
pub enum ShmError {
    /// The size overflows the protocol's `i32`.
    TooLarge,
    /// Making, sizing, writing or sealing the memory failed.
    Io(io::Error),
}

impl fmt::Display for ShmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShmError::TooLarge => formatter.write_str("the buffer is too large for the protocol"),
            ShmError::Io(error) => write!(formatter, "shared memory: {error}"),
        }
    }
}

impl From<io::Error> for ShmError {
    fn from(error: io::Error) -> ShmError {
        ShmError::Io(error)
    }
}

impl From<rustix::io::Errno> for ShmError {
    fn from(error: rustix::io::Errno) -> ShmError {
        ShmError::Io(error.into())
    }
}

/// `image` in a new `wl_buffer`. The pool is destroyed at once; the
/// compositor keeps the memory until the buffer is destroyed.
pub fn buffer<S>(
    shm: &wl_shm::WlShm,
    image: &Image,
    qh: &QueueHandle<S>,
) -> Result<WlBuffer, ShmError>
where
    S: Dispatch<WlShmPool, ()> + Dispatch<WlBuffer, ()> + 'static,
{
    let width = i32::try_from(image.width).map_err(|_| ShmError::TooLarge)?;
    let height = i32::try_from(image.height).map_err(|_| ShmError::TooLarge)?;
    let stride = width.checked_mul(4).ok_or(ShmError::TooLarge)?;
    let size = stride.checked_mul(height).ok_or(ShmError::TooLarge)?;
    let file = File::from(memfd_create(
        "auto-clicker-o01-probe",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?);
    file.set_len(u64::try_from(size).map_err(|_| ShmError::TooLarge)?)?;
    file.write_all_at(&image.bytes, 0)?;
    // The compositor may map it; sealing keeps it from shrinking under it.
    fcntl_add_seals(&file, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;
    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}
