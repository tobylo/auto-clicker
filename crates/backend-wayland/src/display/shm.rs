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

/// `picture` in a new buffer for the screen `name`.
pub(crate) fn buffer(
    shm: &WlShm,
    picture: &Picture,
    name: u32,
    queue: &QueueHandle<Client>,
) -> io::Result<WlBuffer> {
    let too_large = || io::Error::new(io::ErrorKind::InvalidInput, "the buffer is too large");
    let width = i32::try_from(picture.width).map_err(|_| too_large())?;
    let height = i32::try_from(picture.height).map_err(|_| too_large())?;
    let stride = width.checked_mul(4).ok_or_else(too_large)?;
    let size = stride.checked_mul(height).ok_or_else(too_large)?;
    let file = sealed(picture)?;
    let pool = shm.create_pool(file.as_fd(), size, queue, ());
    let buffer = pool.create_buffer(
        0,
        width,
        height,
        stride,
        wl_shm::Format::Argb8888,
        queue,
        name,
    );
    pool.destroy();
    Ok(buffer)
}

/// `picture`'s bytes in an anonymous file, sealed: the compositor maps it,
/// and it can never shrink or grow under it.
fn sealed(picture: &Picture) -> io::Result<File> {
    let file = File::from(memfd_create(
        NAME,
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?);
    let length = u64::try_from(picture.bytes.len()).map_err(io::Error::other)?;
    file.set_len(length)?;
    file.write_all_at(&picture.bytes, 0)?;
    fcntl_add_seals(&file, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use rustix::fs::fcntl_get_seals;

    use super::*;

    #[test]
    fn the_file_holds_the_picture_and_is_sealed_against_any_change_of_size() {
        let picture = Picture {
            width: 2,
            height: 1,
            bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let file = sealed(&picture).unwrap();
        let mut read = vec![0; 8];
        file.read_exact_at(&mut read, 0).unwrap();
        assert_eq!(read, picture.bytes);
        let seals = fcntl_get_seals(&file).unwrap();
        let all = SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL;
        assert!(seals.contains(all), "{seals:?}");
        assert!(file.set_len(4).is_err() && file.set_len(16).is_err());
    }
}
