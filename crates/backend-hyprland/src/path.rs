//! Where Hyprland's socket is: `$XDG_RUNTIME_DIR/hypr/<signature>/.socket.sock`,
//! built only from a signature that is one path component (W01 brief,
//! "Settled by W01d"), so the environment cannot point the daemon at
//! another file.
//!
//! Without a signature that names this display's socket, the daemon looks
//! at the directories under `$XDG_RUNTIME_DIR/hypr` ([`sockets`]), the
//! newest first and at most [`MOST`] of them, each name passed through the
//! same rule.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The socket for `signature` under `runtime`; `None` when `runtime` is not
/// an absolute path or `signature` is not one path component (empty, `.`,
/// `..`, or holding `/` or NUL).
pub(crate) fn socket(runtime: &OsStr, signature: &OsStr) -> Option<PathBuf> {
    let runtime = Path::new(runtime);
    let bytes = signature.as_bytes();
    let one_component = !bytes.is_empty()
        && bytes != b"."
        && bytes != b".."
        && !bytes.contains(&b'/')
        && !bytes.contains(&0);
    (runtime.is_absolute() && one_component)
        .then(|| runtime.join("hypr").join(signature).join(".socket.sock"))
}

/// The most directories under `hypr` looked at: one per Hyprland started
/// since the runtime directory was made, and one a Hyprland that crashed
/// left behind. The running one is among the newest: missing it takes
/// sixteen Hyprlands started after it.
pub(crate) const MOST: usize = 16;

/// The sockets of the instances under `runtime`'s `hypr`, the most recently
/// changed directory first, at most [`MOST`]; none when it cannot be read.
pub(crate) fn sockets(runtime: &OsStr) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(Path::new(runtime).join("hypr")) else {
        return Vec::new();
    };
    let mut dated: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| (changed(&entry), entry.file_name()))
        .collect();
    dated.sort_by(|a, b| b.cmp(a));
    dated
        .iter()
        .take(MOST)
        .filter_map(|(_, name)| socket(runtime, name))
        .collect()
}

/// When `entry` last changed; the oldest time when that cannot be read.
fn changed(entry: &std::fs::DirEntry) -> std::time::SystemTime {
    entry
        .metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_built_only_from_one_path_component() {
        let runtime = OsStr::new("/run/user/1000");
        let signature = "11cbc6c_1763000000_123456789";
        assert_eq!(
            socket(runtime, OsStr::new(signature)),
            Some(PathBuf::from(format!(
                "/run/user/1000/hypr/{signature}/.socket.sock"
            )))
        );
        for bad in ["", ".", "..", "../x", "a/b", "/abs", "a\0b"] {
            assert_eq!(socket(runtime, OsStr::new(bad)), None, "{bad:?}");
        }
        assert_eq!(socket(OsStr::new("run/user"), OsStr::new(signature)), None);
        assert_eq!(socket(OsStr::new(""), OsStr::new(signature)), None);
    }

    #[test]
    fn the_instances_found_are_the_newest_bounded_and_none_without_a_directory() {
        let dir =
            std::env::temp_dir().join(format!("auto-clicker-hypr-dirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(sockets(dir.as_os_str()).is_empty());
        for n in 0..MOST + 4 {
            std::fs::create_dir_all(dir.join("hypr").join(format!("i{n:02}"))).unwrap();
        }
        // The running one is the newest, whatever its name; MOST + 4 = 20.
        let newest = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::open(dir.join("hypr").join("i19"))
            .unwrap()
            .set_modified(newest)
            .unwrap();
        let found = sockets(dir.as_os_str());
        assert_eq!(found.len(), MOST);
        assert!(found.iter().all(|path| path.ends_with(".socket.sock")));
        assert_eq!(found[0], dir.join("hypr/i19/.socket.sock"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
