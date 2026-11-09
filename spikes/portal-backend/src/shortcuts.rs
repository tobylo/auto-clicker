//! GlobalShortcuts: register the three default bindings and print activations.

use std::time::Duration;

use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::Session;
use ashpd::desktop::global_shortcuts::{
    BindShortcutsOptions, GlobalShortcuts, ListShortcutsOptions, NewShortcut, Shortcut,
};
use futures_util::StreamExt;

use crate::log::line;

/// The defaults (map D6), and the triggers a rebind asks for instead.
const BINDINGS: [(&str, &str, &str, &str); 3] = [
    ("clicking", "Start or stop clicking", "CTRL+F6", "CTRL+F9"),
    (
        "recording",
        "Start or stop recording",
        "CTRL+F7",
        "CTRL+F10",
    ),
    (
        "playback",
        "Play or stop the recording",
        "CTRL+F8",
        "CTRL+F11",
    ),
];

pub async fn run(mode: &str, secs: u64) -> Result<(), String> {
    let portal = GlobalShortcuts::new()
        .await
        .map_err(err("GlobalShortcuts"))?;
    line(&format!("GlobalShortcuts version {}", portal.version()));
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(err("CreateSession"))?;
    list(&portal, &session, "before bind").await?;
    if mode != "list" {
        bind(&portal, &session, mode == "rebind").await?;
        list(&portal, &session, "after bind").await?;
    }
    listen(&portal, Duration::from_secs(secs)).await?;
    session.close().await.map_err(err("Session.Close"))
}

fn err(call: &'static str) -> impl Fn(ashpd::Error) -> String {
    move |e| format!("{call}: {e}")
}

fn shortcuts(alternate: bool) -> Vec<NewShortcut> {
    BINDINGS
        .iter()
        .map(|(id, description, default, other)| {
            let trigger = if alternate { *other } else { *default };
            NewShortcut::new(*id, *description).preferred_trigger(trigger)
        })
        .collect()
}

async fn bind(
    portal: &GlobalShortcuts,
    session: &Session<GlobalShortcuts>,
    alternate: bool,
) -> Result<(), String> {
    let label = if alternate { "rebind" } else { "bind" };
    let bound = portal
        .bind_shortcuts(
            session,
            &shortcuts(alternate),
            None,
            BindShortcutsOptions::default(),
        )
        .await
        .map_err(err("BindShortcuts"))?
        .response()
        .map_err(err("BindShortcuts response"))?;
    print(label, bound.shortcuts());
    Ok(())
}

async fn list(
    portal: &GlobalShortcuts,
    session: &Session<GlobalShortcuts>,
    label: &str,
) -> Result<(), String> {
    let listed = portal
        .list_shortcuts(session, ListShortcutsOptions::default())
        .await
        .map_err(err("ListShortcuts"))?
        .response()
        .map_err(err("ListShortcuts response"))?;
    print(&format!("list {label}"), listed.shortcuts());
    Ok(())
}

fn print(label: &str, shortcuts: &[Shortcut]) {
    line(&format!("{label}: {} shortcuts", shortcuts.len()));
    for shortcut in shortcuts {
        line(&format!(
            "  {} = {:?} ({})",
            shortcut.id(),
            shortcut.trigger_description(),
            shortcut.description()
        ));
    }
}

/// Prints every activation, deactivation and change until `duration` ends.
async fn listen(portal: &GlobalShortcuts, duration: Duration) -> Result<(), String> {
    let mut activated = portal.receive_activated().await.map_err(err("Activated"))?;
    let mut deactivated = portal
        .receive_deactivated()
        .await
        .map_err(err("Deactivated"))?;
    let mut changed = portal
        .receive_shortcuts_changed()
        .await
        .map_err(err("ShortcutsChanged"))?;
    line(&format!("listening for {} s", duration.as_secs()));
    let end = tokio::time::sleep(duration);
    tokio::pin!(end);
    loop {
        tokio::select! {
            () = &mut end => return Ok(()),
            Some(a) = activated.next() => line(&format!("activated {} at {:?}", a.shortcut_id(), a.timestamp())),
            Some(d) = deactivated.next() => line(&format!("deactivated {} at {:?}", d.shortcut_id(), d.timestamp())),
            Some(c) = changed.next() => print("changed", c.shortcuts()),
        }
    }
}
