//! A picture in shared memory: an anonymous file from `memfd_create`,
//! written with `pwrite` and sealed, so neither a named file under
//! `XDG_RUNTIME_DIR` nor `mmap` and its `unsafe` is needed (0005, point 11).
//! The file is closed once the pool is made; the compositor keeps the memory
//! until the buffer is destroyed.

use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm::{self, WlShm};

use super::paint::Picture;
use crate::client::Client;

/// The anonymous file's name, which only `/proc` shows.
const NAME: &str = "auto-clicker-on-screen";

/// `picture` in a new buffer.
pub(crate) fn buffer(
    shm: &WlShm,
    picture: &Picture,
    queue: &QueueHandle<Client>,
) -> io::Result<WlBuffer> {
    let too_large = || io::Error::new(io::ErrorKind::InvalidInput, "the buffer is too large");
    let width = i32::try_from(picture.width).map_err(|_| too_large())?;
    let height = i32::try_from(picture.height).map_err(|_| too_large())?;
    let stride = width.checked_mul(4).ok_or_else(too_large)?;
    let size = stride.checked_mul(height).ok_or_else(too_large)?;
    let file = File::from(memfd_create(
        NAME,
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?);
    file.set_len(u64::try_from(size).map_err(|_| too_large())?)?;
    file.write_all_at(&picture.bytes, 0)?;
    // The compositor maps it: sealed, it can never shrink under it.
    fcntl_add_seals(&file, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;
    let pool = shm.create_pool(file.as_fd(), size, queue, ());
    let buffer = pool.create_buffer(
        0,
        width,
        height,
        stride,
        wl_shm::Format::Argb8888,
        queue,
        (),
    );
    pool.destroy();
    Ok(buffer)
}
