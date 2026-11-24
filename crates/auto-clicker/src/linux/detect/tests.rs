//! Detection for each combination of access: `/dev/uinput` writable or not,
//! event nodes readable, unreadable or missing, and portals present, too
//! old or absent, or not answering.

use std::cell::Cell;
use std::io;

use super::*;

/// How opening `/dev/uinput` goes.
#[derive(Clone, Copy, Debug)]
enum Uinput {
    Writable,
    Denied,
    Missing,
    /// A failure access does not explain, such as a device that could not
    /// be set up.
    Failed,
}

impl Uinput {
    fn open(self) -> Result<(), InputError> {
        let err = |kind: io::ErrorKind| io::Error::from(kind);
        match self {
            Uinput::Writable => Ok(()),
            Uinput::Denied => Err(InputError::NoPermission(err(
                io::ErrorKind::PermissionDenied,
            ))),
            Uinput::Missing => Err(InputError::UinputMissing(err(io::ErrorKind::NotFound))),
            Uinput::Failed => Err(InputError::CreateDevice(err(io::ErrorKind::Other))),
        }
    }
}

/// How a portal check goes.
#[derive(Clone, Copy, Debug)]
enum Portal {
    /// Present at version 2.
    Offered,
    /// Absent, or older than version 2.
    Unavailable,
    /// The bus or the portal did not answer in time.
    Silent,
}

impl Portal {
    fn check(self) -> Result<(), InputError> {
        match self {
            Portal::Offered => Ok(()),
            Portal::Unavailable => Err(InputError::PortalUnavailable("version 1".to_owned())),
            Portal::Silent => Err(InputError::Portal("no answer within 10 s".to_owned())),
        }
    }
}

/// What a choice of output came to, without the error's text.
#[derive(Debug, PartialEq, Eq)]
enum Picked {
    Kernel,
    /// The kernel route, reporting why its devices did not open.
    KernelFailed,
    Portal,
    /// The portal route, reporting what its check found.
    PortalProblem,
}

/// The output `route` picks, and which checks ran.
async fn output(route: OutputRoute, uinput: Uinput, portal: Portal) -> (Picked, bool, bool) {
    let (opened, probed) = (Cell::new(false), Cell::new(false));
    let open = || {
        opened.set(true);
        uinput.open()
    };
    let probe = || {
        probed.set(true);
        std::future::ready(portal.check())
    };
    let picked = match choose_output(route, open, probe).await {
        Output::Kernel(Ok(())) => Picked::Kernel,
        Output::Kernel(Err(_)) => Picked::KernelFailed,
        Output::Portal(None) => Picked::Portal,
        Output::Portal(Some(_)) => Picked::PortalProblem,
    };
    (picked, opened.get(), probed.get())
}

#[tokio::test]
async fn auto_output_takes_uinput_where_writable_and_the_portal_otherwise() {
    use Picked::{Kernel, KernelFailed, Portal as OnPortal, PortalProblem};
    let cases = [
        // uinput, portal: picked, whether the portal was asked.
        (Uinput::Writable, Portal::Offered, Kernel, false),
        (Uinput::Writable, Portal::Unavailable, Kernel, false),
        (Uinput::Writable, Portal::Silent, Kernel, false),
        (Uinput::Denied, Portal::Offered, OnPortal, true),
        (Uinput::Denied, Portal::Unavailable, KernelFailed, true),
        (Uinput::Denied, Portal::Silent, PortalProblem, true),
        (Uinput::Missing, Portal::Offered, OnPortal, true),
        (Uinput::Missing, Portal::Unavailable, KernelFailed, true),
        (Uinput::Missing, Portal::Silent, PortalProblem, true),
        // A failure access does not explain stays on the kernel route,
        // which tries again at the next start.
        (Uinput::Failed, Portal::Offered, KernelFailed, false),
    ];
    for (uinput, portal, picked, asked) in cases {
        let got = output(OutputRoute::Auto, uinput, portal).await;
        assert_eq!(got, (picked, true, asked), "{uinput:?} {portal:?}");
    }
}

#[tokio::test]
async fn an_explicit_kernel_output_is_used_as_written_and_says_why_it_fails() {
    let cases = [
        (Uinput::Writable, Picked::Kernel),
        (Uinput::Denied, Picked::KernelFailed),
        (Uinput::Missing, Picked::KernelFailed),
    ];
    for (uinput, picked) in cases {
        // A portal that would serve is never asked.
        let got = output(OutputRoute::Kernel, uinput, Portal::Offered).await;
        assert_eq!(got, (picked, true, false), "{uinput:?}");
    }
}

#[tokio::test]
async fn an_explicit_portal_output_is_used_as_written_and_says_why_it_fails() {
    let cases = [
        (Portal::Offered, Picked::Portal),
        (Portal::Unavailable, Picked::PortalProblem),
        (Portal::Silent, Picked::PortalProblem),
    ];
    for (portal, picked) in cases {
        // Writable uinput is never opened.
        let got = output(OutputRoute::Portal, Uinput::Writable, portal).await;
        assert_eq!(got, (picked, false, true), "{portal:?}");
    }
}

/// What a look at the event nodes finds.
#[derive(Clone, Copy, Debug)]
enum Nodes {
    /// A keyboard with the hotkey keys is readable.
    Readable,
    /// A node may not be opened, and no hotkey keyboard is readable: a
    /// readable gamepad beside an unreadable keyboard looks like this.
    Unreadable,
    /// No keyboard: none plugged in, or every node went away.
    Missing,
    /// The directory could not be listed.
    Unlisted,
}

impl Nodes {
    fn status(self) -> HotkeyStatus {
        match self {
            Nodes::Readable => HotkeyStatus::Available,
            Nodes::Unreadable => HotkeyStatus::Unavailable(Unavailable::NoPermission),
            Nodes::Missing => HotkeyStatus::Unavailable(Unavailable::NoKeyboard),
            Nodes::Unlisted => HotkeyStatus::Unavailable(Unavailable::WatchFailed(
                "permission denied".to_owned().into(),
            )),
        }
    }
}

/// The reader `route` picks, and which checks ran.
async fn hotkeys(route: HotkeyRoute, nodes: Nodes, portal: Portal) -> (InputReader, bool, bool) {
    let (looked, asked) = (Cell::new(false), Cell::new(false));
    let keyboards = || {
        looked.set(true);
        nodes.status()
    };
    let offered = || {
        asked.set(true);
        std::future::ready(portal.check())
    };
    let reader = choose_hotkeys(route, keyboards, offered).await;
    (reader, looked.get(), asked.get())
}

#[tokio::test]
async fn auto_hotkeys_read_the_keyboards_unless_they_may_not_be_read() {
    use InputReader::{Desktop, Devices};
    let cases = [
        // nodes, portal: reader, whether the portal was asked.
        (Nodes::Readable, Portal::Offered, Devices, false),
        (Nodes::Readable, Portal::Unavailable, Devices, false),
        (Nodes::Unreadable, Portal::Offered, Desktop, true),
        (Nodes::Unreadable, Portal::Silent, Desktop, true),
        // No portal to fall back on: the keyboards' reader says why.
        (Nodes::Unreadable, Portal::Unavailable, Devices, true),
        // No keyboard yet: the reader waits for one.
        (Nodes::Missing, Portal::Offered, Devices, false),
        // The reader could not watch for keyboards: it would stop at once.
        (Nodes::Unlisted, Portal::Offered, Desktop, true),
        (Nodes::Unlisted, Portal::Unavailable, Devices, true),
    ];
    for (nodes, portal, reader, asked) in cases {
        let got = hotkeys(HotkeyRoute::Auto, nodes, portal).await;
        assert_eq!(got, (reader, true, asked), "{nodes:?} {portal:?}");
    }
}

#[tokio::test]
async fn explicit_hotkeys_are_used_as_written_without_a_check() {
    for nodes in [Nodes::Readable, Nodes::Unreadable, Nodes::Missing] {
        let kernel = hotkeys(HotkeyRoute::Kernel, nodes, Portal::Offered).await;
        let desktop = hotkeys(HotkeyRoute::Portal, nodes, Portal::Unavailable).await;
        assert_eq!(kernel, (InputReader::Devices, false, false), "{nodes:?}");
        assert_eq!(desktop, (InputReader::Desktop, false, false), "{nodes:?}");
    }
}

#[test]
fn the_routes_picked_are_named_as_the_tray_names_them() {
    let denied = || Uinput::Denied.open().unwrap_err();
    let cases = [
        (
            Output::Kernel(Err(denied())),
            InputReader::Devices,
            Route::Kernel,
            Route::Kernel,
        ),
        (
            Output::Kernel(Err(denied())),
            InputReader::Desktop,
            Route::Kernel,
            Route::Portal,
        ),
        (
            Output::Portal(None),
            InputReader::Devices,
            Route::Portal,
            Route::Kernel,
        ),
        (
            Output::Portal(Some(denied())),
            InputReader::Desktop,
            Route::Portal,
            Route::Portal,
        ),
    ];
    for (output, hotkeys, clicks, keys) in cases {
        let routes = Detected { output, hotkeys }.routes();
        assert_eq!(
            routes,
            Routes {
                output: clicks,
                hotkeys: keys
            }
        );
    }
}

#[test]
fn only_devices_the_kernel_route_opened_are_left_out_of_the_keyboard_check() {
    let denied = || Uinput::Denied.open().unwrap_err();
    assert_eq!(Output::Kernel(Ok(7)).kept(), Some(&7));
    assert_eq!(Output::<u8>::Kernel(Err(denied())).kept(), None);
    assert_eq!(Output::<u8>::Portal(None).kept(), None);
}
