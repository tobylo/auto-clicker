//! Creating the virtual devices, and who they are once created.

use std::io;
use std::path::{Path, PathBuf};

use auto_clicker_core::input::{Button, InputError, Key};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, InputId, KeyCode, RelativeAxisCode,
    UinputAbsSetup,
};

use crate::error::open_error;

/// The device names. Kept stable: KWin stores per-device settings by name,
/// vendor and product. Never used to recognise the devices; see
/// [`DeviceIdentity`].
pub const MOUSE_NAME: &str = "auto-clicker virtual mouse";
pub const KEYBOARD_NAME: &str = "auto-clicker virtual keyboard";

/// The vendor id on every virtual device ("ac1c", auto clicker). Anyone can
/// create a uinput device with the same ids, so they identify the device
/// type, not this process's devices.
pub const VENDOR: u16 = 0xac1c;
pub const PRODUCT_MOUSE: u16 = 0x0001;
pub const PRODUCT_KEYBOARD: u16 = 0x0002;
const VERSION: u16 = 1;

/// The largest absolute coordinate on each axis; 0 is the smallest.
pub const ABSOLUTE_MAX: u16 = u16::MAX;

/// Which virtual device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceRole {
    Mouse,
    Keyboard,
}

impl DeviceRole {
    pub fn name(self) -> &'static str {
        match self {
            DeviceRole::Mouse => MOUSE_NAME,
            DeviceRole::Keyboard => KEYBOARD_NAME,
        }
    }

    pub fn product(self) -> u16 {
        match self {
            DeviceRole::Mouse => PRODUCT_MOUSE,
            DeviceRole::Keyboard => PRODUCT_KEYBOARD,
        }
    }
}

/// A virtual device this process created.
///
/// Recognise the device by `syspath`: the kernel's input node
/// (`/sys/devices/virtual/input/inputN`) is unique while the device exists,
/// unlike its name or ids, which any other uinput user can copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub role: DeviceRole,
    pub syspath: PathBuf,
}

/// Creates one virtual device and reads back its identity.
pub(crate) fn create(role: DeviceRole) -> Result<(VirtualDevice, DeviceIdentity), InputError> {
    let builder = VirtualDevice::builder().map_err(open_error)?;
    let mut device = configure(builder, role)
        .and_then(|builder| builder.build())
        .map_err(InputError::CreateDevice)?;
    let syspath = device
        .get_syspath()
        .and_then(checked_syspath)
        .map_err(InputError::CreateDevice)?;
    Ok((device, DeviceIdentity { role, syspath }))
}

fn configure(
    builder: VirtualDeviceBuilder<'static>,
    role: DeviceRole,
) -> io::Result<VirtualDeviceBuilder<'static>> {
    let builder = builder.name(role.name()).input_id(InputId::new(
        BusType::BUS_VIRTUAL,
        VENDOR,
        role.product(),
        VERSION,
    ));
    match role {
        DeviceRole::Mouse => configure_mouse(builder),
        DeviceRole::Keyboard => builder.with_keys(&keyboard_keys()),
    }
}

/// Buttons, relative motion, the wheel and absolute axes on one device: udev
/// tags that an ordinary mouse (`ID_INPUT_MOUSE`), as it does a relative-only
/// one.
fn configure_mouse(
    builder: VirtualDeviceBuilder<'static>,
) -> io::Result<VirtualDeviceBuilder<'static>> {
    let mut buttons = AttributeSet::<KeyCode>::new();
    for button in Button::ALL {
        buttons.insert(KeyCode(button.code()));
    }
    let mut axes = AttributeSet::<RelativeAxisCode>::new();
    for axis in [
        RelativeAxisCode::REL_X,
        RelativeAxisCode::REL_Y,
        RelativeAxisCode::REL_WHEEL,
    ] {
        axes.insert(axis);
    }
    let range = AbsInfo::new(0, 0, i32::from(ABSOLUTE_MAX), 0, 0, 0);
    builder
        .with_keys(&buttons)?
        .with_relative_axes(&axes)?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, range))?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, range))
}

/// Exactly the codes a [`Key`] may hold, so every key the backend is given
/// can be emitted, and no `BTN_*` code makes udev take the keyboard for a
/// joystick.
fn keyboard_keys() -> AttributeSet<KeyCode> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in Key::CODES {
        keys.insert(KeyCode(code));
    }
    keys
}

/// Accepts the syspath only if it names a kernel input node that exists. The
/// kernel always reports a name (`uinput_str_to_user` returns strlen + 1),
/// so a failure here means something unexpected, and creation fails.
fn checked_syspath(path: PathBuf) -> io::Result<PathBuf> {
    if !is_input_node(&path) {
        return Err(io::Error::other(format!(
            "unexpected sysfs path for a new virtual device: {}",
            path.display()
        )));
    }
    path.try_exists()?
        .then_some(path)
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
}

/// Whether the last component is `input` followed by digits.
fn is_input_node(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("input"))
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VirtualDeviceBuilder::build` panics on a name of 80 bytes or more
    /// (`UINPUT_MAX_NAME_SIZE`, NUL included).
    #[test]
    fn names_fit_the_uinput_limit() {
        for role in [DeviceRole::Mouse, DeviceRole::Keyboard] {
            assert!(role.name().len() < 79, "{role:?} name too long");
        }
    }

    #[test]
    fn keyboard_declares_exactly_the_key_range() {
        let keys = keyboard_keys();
        let codes: Vec<u16> = keys.iter().map(|k| k.0).collect();
        assert_eq!(codes, Key::CODES.collect::<Vec<_>>());
        assert!(codes.iter().all(|&code| code < 0x100), "no BTN_* codes");
    }

    #[test]
    fn devices_have_distinct_products() {
        assert_ne!(DeviceRole::Mouse.product(), DeviceRole::Keyboard.product());
    }

    #[test]
    fn input_nodes_are_recognised() {
        assert!(is_input_node(Path::new(
            "/sys/devices/virtual/input/input42"
        )));
        assert!(!is_input_node(Path::new(
            "/sys/devices/virtual/input/input"
        )));
        assert!(!is_input_node(Path::new(
            "/sys/devices/virtual/input/input4x"
        )));
        assert!(!is_input_node(Path::new(
            "/sys/devices/virtual/input/event3"
        )));
        assert!(!is_input_node(Path::new("/sys/devices/virtual/input/")));
    }

    #[test]
    fn syspath_must_name_an_existing_input_node() {
        let missing = PathBuf::from("/nonexistent/auto-clicker-test/input7");
        assert_eq!(
            checked_syspath(missing).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let not_a_node = PathBuf::from("/sys/devices/virtual/input");
        assert_eq!(
            checked_syspath(not_a_node).unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }
}
