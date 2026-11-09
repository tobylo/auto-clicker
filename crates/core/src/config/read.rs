//! Reading the config file without blocking or reading without end.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::Problem;

/// The largest config file read: far more than any real one needs.
pub(super) const MAX_SIZE: u64 = 64 * 1024;

/// The file's bytes, or `None` when there is no file.
///
/// Opens without blocking, so a FIFO with no writer cannot stall the program,
/// and reads only from a regular file, checked on the opened handle, so a
/// device such as `/dev/zero` is never read. Reads at most [`MAX_SIZE`] bytes.
pub(super) fn read_config(path: &Path) -> Result<Option<Vec<u8>>, Problem> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Problem::Read(err)),
    };
    if !file.metadata().map_err(Problem::Read)?.is_file() {
        return Err(invalid("the path is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_SIZE.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(Problem::Read)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SIZE {
        return Err(invalid("the file is larger than 64 KiB"));
    }
    Ok(Some(bytes))
}

fn invalid(reason: &str) -> Problem {
    Problem::Invalid {
        line: None,
        reason: reason.to_owned(),
    }
}
