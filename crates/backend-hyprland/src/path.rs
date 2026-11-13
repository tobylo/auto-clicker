//! Where Hyprland's socket is: `$XDG_RUNTIME_DIR/hypr/<signature>/.socket.sock`,
//! built only from a signature that is one path component (W01 brief,
//! "Settled by W01d"), so the environment cannot point the daemon at
//! another file.

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
}
