//! Calls to KWin: following who owns `org.kde.KWin`, and loading, running
//! and unloading scripts through its `/Scripting` object.
//!
//! Every call goes to the owner's unique name, so one load's calls reach one
//! KWin, and carries `NO_AUTO_START`, so no call ever starts KWin or
//! anything else through D-Bus activation. Each has its own deadline
//! ([`bounded`]).

use std::future::{Future, ready};
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use zbus::Connection;

use crate::script::Script;
use zbus::export::serde::{Deserialize, Serialize};
use zbus::fdo::{self, DBusProxy};
use zbus::names::{BusName, WellKnownName};
use zbus::proxy::{Builder, CacheProperties, MethodFlags};
use zbus::zvariant::{DynamicType, Type};

/// KWin's well-known name.
pub(crate) const KWIN: &str = "org.kde.KWin";
/// How long one call to KWin or the bus may take.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(2);

const SCRIPTING_PATH: &str = "/Scripting";
const SCRIPTING: &str = "org.kde.kwin.Scripting";
const SCRIPT: &str = "org.kde.kwin.Script";

/// Why a call failed, as the log says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallError(pub(crate) String);

impl From<zbus::Error> for CallError {
    fn from(err: zbus::Error) -> CallError {
        CallError(err.to_string())
    }
}

impl From<fdo::Error> for CallError {
    fn from(err: fdo::Error) -> CallError {
        CallError(err.to_string())
    }
}

/// The owners `org.kde.KWin` gets, in order; `None` when it has none.
pub(crate) type Owners = Pin<Box<dyn Stream<Item = Option<String>>>>;

/// What the watcher needs from KWin. The daemon's is [`Bus`]; tests use a
/// fake.
pub(crate) trait Scripting {
    /// Follows `org.kde.KWin`'s owner: subscribes to its changes first, then
    /// reads who owns it now, so no change between the two is missed.
    async fn follow_owner(&self) -> Result<(Option<String>, Owners), CallError>;

    /// Unloads whatever is loaded under `script`'s plugin name.
    async fn unload(&self, owner: &str, script: Script) -> Result<(), CallError>;

    /// Loads the file at `path` under `script`'s plugin name; its id.
    async fn load(&self, owner: &str, path: &Path, script: Script) -> Result<i32, CallError>;

    /// Runs the loaded script `id`. KWin answers once the script has run.
    async fn run(&self, owner: &str, id: i32) -> Result<(), CallError>;
}

/// `call`, failing when it takes longer than [`CALL_TIMEOUT`].
pub(crate) async fn bounded<T>(
    call: impl Future<Output = Result<T, CallError>>,
) -> Result<T, CallError> {
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(result) => result,
        Err(_) => Err(CallError("KWin did not answer".to_owned())),
    }
}

/// `org.kde.KWin` as a bus name; [`KWIN`] is a valid one.
fn kwin_name() -> BusName<'static> {
    BusName::WellKnown(WellKnownName::from_static_str_unchecked(KWIN))
}

/// An object of one KWin, and one of its interfaces.
struct Target<'a> {
    owner: &'a str,
    path: &'a str,
    interface: &'a str,
}

impl Target<'_> {
    /// `owner`'s `/Scripting`.
    fn scripting(owner: &str) -> Target<'_> {
        Target {
            owner,
            path: SCRIPTING_PATH,
            interface: SCRIPTING,
        }
    }
}

/// KWin on the session bus.
pub(crate) struct Bus(pub(crate) Connection);

impl Bus {
    /// Calls `method` on `target`, without starting anything.
    async fn call<R>(
        &self,
        target: Target<'_>,
        method: &str,
        body: &(impl Serialize + DynamicType),
    ) -> Result<R, CallError>
    where
        R: for<'d> Deserialize<'d> + Type,
    {
        let proxy: zbus::Proxy<'_> = Builder::new(&self.0)
            .destination(target.owner)?
            .path(target.path)?
            .interface(target.interface)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let reply = proxy
            .call_with_flags(method, MethodFlags::NoAutoStart.into(), body)
            .await?;
        reply.ok_or_else(|| CallError(format!("{method} gave no reply")))
    }
}

#[cfg(test)]
impl Bus {
    /// Whether KWin has a script loaded under `script`'s plugin name.
    pub(crate) async fn is_loaded(&self, owner: &str, script: Script) -> Result<bool, CallError> {
        let plugin = script.plugin();
        self.call(Target::scripting(owner), "isScriptLoaded", &(plugin,))
            .await
    }
}

impl Scripting for Bus {
    async fn follow_owner(&self) -> Result<(Option<String>, Owners), CallError> {
        let dbus = DBusProxy::new(&self.0).await?;
        let changes = dbus
            .receive_name_owner_changed_with_args(&[(0, KWIN)])
            .await?;
        let owners = changes.filter_map(|signal| {
            let owner = signal
                .args()
                .ok()
                .map(|args| args.new_owner().as_ref().map(ToString::to_string));
            ready(owner)
        });
        let owner = match dbus.get_name_owner(kwin_name()).await {
            Ok(owner) => Some(owner.to_string()),
            Err(fdo::Error::NameHasNoOwner(_)) => None,
            Err(err) => return Err(err.into()),
        };
        Ok((owner, Box::pin(owners)))
    }

    async fn unload(&self, owner: &str, script: Script) -> Result<(), CallError> {
        let plugin = script.plugin();
        let _was_loaded: bool = self
            .call(Target::scripting(owner), "unloadScript", &(plugin,))
            .await?;
        Ok(())
    }

    async fn load(&self, owner: &str, path: &Path, script: Script) -> Result<i32, CallError> {
        let body = (utf8(path)?, script.plugin());
        self.call(Target::scripting(owner), "loadScript", &body)
            .await
    }

    async fn run(&self, owner: &str, id: i32) -> Result<(), CallError> {
        let path = format!("{SCRIPTING_PATH}/Script{id}");
        let target = Target {
            owner,
            path: &path,
            interface: SCRIPT,
        };
        self.call(target, "run", &()).await
    }
}

/// The path as D-Bus carries it: UTF-8 only.
fn utf8(path: &Path) -> Result<&str, CallError> {
    path.to_str()
        .ok_or_else(|| CallError(format!("{} is not UTF-8", path.display())))
}
