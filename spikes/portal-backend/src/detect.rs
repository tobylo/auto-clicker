//! Which input routes this session offers, one result per capability.
//!
//! Each check uses the thing it reports on: it opens the device node (and
//! would keep that handle in the daemon) or calls the portal, rather than
//! reading permission bits that a later open could contradict.

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;

use ashpd::desktop::global_shortcuts::GlobalShortcuts;
use ashpd::desktop::input_capture::InputCapture;
use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop};

use crate::log::line;

pub async fn run() -> Result<(), String> {
    let uinput = uinput();
    let evdev = evdev();
    let portals = Portals::query().await;
    line(&format!("uinput: {uinput}"));
    line(&format!("evdev: {evdev}"));
    portals.print();
    line(&format!("output: {}", output_route(&uinput, &portals)));
    line(&format!("hotkeys: {}", hotkey_route(&evdev, &portals)));
    line(&format!("recording: {}", recording_route(&evdev)));
    Ok(())
}

/// The result of opening `/dev/uinput` for writing.
enum Access {
    Granted,
    Denied,
    Missing,
    Failed(String),
}

impl std::fmt::Display for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Access::Granted => write!(f, "writable"),
            Access::Denied => write!(f, "permission denied"),
            Access::Missing => write!(f, "no such device"),
            Access::Failed(err) => write!(f, "failed: {err}"),
        }
    }
}

fn access(result: std::io::Result<File>) -> Access {
    match result {
        Ok(_) => Access::Granted,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => Access::Denied,
        Err(err) if err.kind() == ErrorKind::NotFound => Access::Missing,
        Err(err) => Access::Failed(err.to_string()),
    }
}

fn uinput() -> Access {
    access(OpenOptions::new().write(true).open("/dev/uinput"))
}

/// How many event nodes could be opened for reading, out of how many.
struct Evdev {
    readable: usize,
    denied: usize,
    other: usize,
}

impl std::fmt::Display for Evdev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.readable + self.denied + self.other;
        write!(
            f,
            "{} of {total} event nodes readable ({} denied, {} other errors)",
            self.readable, self.denied, self.other
        )
    }
}

fn evdev() -> Evdev {
    let mut counts = Evdev {
        readable: 0,
        denied: 0,
        other: 0,
    };
    for path in event_nodes() {
        match access(File::open(&path)) {
            Access::Granted => counts.readable += 1,
            Access::Denied => counts.denied += 1,
            _ => counts.other += 1,
        }
    }
    counts
}

fn event_nodes() -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_event_node(path))
        .collect()
}

fn is_event_node(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("event"))
}

/// What the portal frontend reports, or why it could not be asked.
struct Portals {
    remote_desktop: Result<(u32, DeviceType), String>,
    global_shortcuts: Result<u32, String>,
    input_capture: Result<u32, String>,
}

impl Portals {
    async fn query() -> Portals {
        Portals {
            remote_desktop: remote_desktop().await,
            global_shortcuts: global_shortcuts().await,
            input_capture: input_capture().await,
        }
    }

    fn print(&self) {
        line(&format!("RemoteDesktop: {:?}", self.remote_desktop));
        line(&format!("GlobalShortcuts: {:?}", self.global_shortcuts));
        line(&format!("InputCapture: {:?}", self.input_capture));
    }
}

async fn remote_desktop() -> Result<(u32, DeviceType), String> {
    let proxy = RemoteDesktop::new().await.map_err(|e| e.to_string())?;
    let types = proxy
        .available_device_types()
        .await
        .map_err(|e| e.to_string())?;
    Ok((proxy.version(), types))
}

async fn global_shortcuts() -> Result<u32, String> {
    let proxy = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
    Ok(proxy.version())
}

async fn input_capture() -> Result<u32, String> {
    let proxy = InputCapture::new().await.map_err(|e| e.to_string())?;
    let caps = proxy
        .supported_capabilities()
        .await
        .map_err(|e| e.to_string())?;
    line(&format!("InputCapture capabilities: {caps:?}"));
    Ok(proxy.version())
}

fn output_route(uinput: &Access, portals: &Portals) -> &'static str {
    let needed = DeviceType::KEYBOARD | DeviceType::POINTER;
    match (uinput, &portals.remote_desktop) {
        (Access::Granted, _) => "kernel (uinput)",
        // ConnectToEIS arrived in RemoteDesktop version 2.
        (_, Ok((version, types))) if *version >= 2 && types.contains(needed) => {
            "portal (RemoteDesktop + libei)"
        }
        _ => "none",
    }
}

fn hotkey_route(evdev: &Evdev, portals: &Portals) -> &'static str {
    match (evdev.readable, &portals.global_shortcuts) {
        (1.., _) => "kernel (evdev)",
        (0, Ok(_)) => "portal (GlobalShortcuts)",
        (0, Err(_)) => "none",
    }
}

fn recording_route(evdev: &Evdev) -> &'static str {
    // InputCapture takes input away from the focused window (see the
    // decision record), so it is not a recording route.
    if evdev.readable > 0 {
        "kernel (evdev)"
    } else {
        "none"
    }
}
