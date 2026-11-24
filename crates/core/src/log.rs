//! Where the program's diagnostic lines go: standard error, which is the
//! journal on Linux, until [`to_file`] opens a log file, which a program
//! without a console needs (Windows, M5 brief decision 13).
//!
//! A line that cannot be written is dropped: logging never fails, blocks
//! on anything but the write itself, or panics the program. What is logged
//! is what Linux writes to the journal, and never a keystroke.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::fs::create_private_dir_all;

/// How large the log file grows before it becomes the `.1` file and a new
/// one starts.
pub const LOG_CAP: u64 = 1024 * 1024;

/// The open log file; none while lines go to standard error.
static FILE: Mutex<Option<LogFile>> = Mutex::new(None);

/// Writes `args` as one line to the log file, or to standard error when
/// none is open. A failed write is dropped.
pub fn line(args: fmt::Arguments<'_>) {
    let text = format!("{args}\n");
    let mut file = FILE.lock().unwrap_or_else(PoisonError::into_inner);
    match file.as_mut() {
        Some(file) => file.write_line(&text),
        None => {
            let _ = io::stderr().write_all(text.as_bytes());
        }
    }
}

/// Sends every later line to the file at `path`, creating its folder
/// (private to its owner on Linux) if needed, and appending to what is
/// there; panic messages go there too.
///
/// # Errors
/// The folder or the file cannot be created or opened; lines then keep
/// going to standard error.
pub fn to_file(path: &Path) -> io::Result<()> {
    let file = LogFile::open(path.to_owned(), LOG_CAP)?;
    *FILE.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
    std::panic::set_hook(Box::new(|panic| {
        line(format_args!("auto-clicker: {panic}"))
    }));
    Ok(())
}

/// A log file that becomes its `.1` file once a line would take it past
/// its cap.
struct LogFile {
    path: PathBuf,
    file: File,
    /// The file's size, as far as this program has written it.
    size: u64,
    cap: u64,
}

impl LogFile {
    fn open(path: PathBuf, cap: u64) -> io::Result<LogFile> {
        if let Some(dir) = path.parent() {
            create_private_dir_all(dir)?;
        }
        let file = append(&path)?;
        let size = file.metadata()?.len();
        Ok(LogFile {
            path,
            file,
            size,
            cap,
        })
    }

    /// Writes `text`, after starting a new file when it would take this one
    /// past the cap. A failed write is dropped, and counted as written: one
    /// that wrote part of the line still grew the file, and counting too
    /// much only starts the next file sooner.
    fn write_line(&mut self, text: &str) {
        let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
        if self.size.saturating_add(len) > self.cap {
            self.start_new();
        }
        let _ = self.file.write_all(text.as_bytes());
        self.size = self.size.saturating_add(len);
    }

    /// Renames the file to `.1`, replacing the one before, and starts a new
    /// one. No file at the path means it has been moved already: by an
    /// earlier rename whose new file could not be opened, or by someone
    /// else. When either step fails, lines keep going to the file open now,
    /// and the next line tries again.
    fn start_new(&mut self) {
        match std::fs::rename(&self.path, rotated(&self.path)) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return,
        }
        if let Ok(file) = append(&self.path) {
            self.file = file;
            self.size = 0;
        }
    }
}

/// `path` opened for appending, created if missing.
fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Where a full log file goes: `auto-clicker.log.1`.
fn rotated(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests;
