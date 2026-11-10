//! InputCapture: a pointer barrier along the top edge, then what is received.
//!
//! Prints event kinds and counts only, never key codes: a spike must not log
//! keystrokes (AGENTS.md, "Hard rules").

use std::collections::BTreeMap;
use std::num::NonZero;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use ashpd::desktop::Session;
use ashpd::desktop::input_capture::{
    Barrier, Capabilities, ConnectToEISOptions, CreateSessionOptions, EnableOptions,
    GetZonesOptions, InputCapture, ReleaseOptions, SetPointerBarriersOptions, Zones,
};
use futures_util::StreamExt;
use reis::ei;
use reis::event::{DeviceCapability, EiEvent};

use crate::log::line;

/// How long a capture lasts before the spike hands input back.
const HAND_BACK: Duration = Duration::from_secs(3);

pub async fn run(secs: u64) -> Result<(), String> {
    let portal = InputCapture::new().await.map_err(err("InputCapture"))?;
    line(&format!("InputCapture version {}", portal.version()));
    let options = CreateSessionOptions::default()
        .set_capabilities(Capabilities::KEYBOARD | Capabilities::POINTER);
    let (session, caps) = portal
        .create_session(None, options)
        .await
        .map_err(err("CreateSession"))?;
    line(&format!("session capabilities {caps:?}"));
    let fd = portal
        .connect_to_eis(&session, ConnectToEISOptions::default())
        .await
        .map_err(err("ConnectToEIS"))?;
    barriers(&portal, &session).await?;
    let context = ei::Context::new(UnixStream::from(fd)).map_err(|e| format!("ei: {e}"))?;
    let (connection, events) = context
        .handshake_tokio(
            "auto-clicker-x02-capture",
            ei::handshake::ContextType::Receiver,
        )
        .await
        .map_err(|e| format!("ei handshake: {e}"))?;
    portal
        .enable(&session, EnableOptions::default())
        .await
        .map_err(err("Enable"))?;
    line("enabled: push the pointer against the top edge");
    let receiver = Receiver { connection, events };
    let result = watch(&portal, &session, receiver, Duration::from_secs(secs)).await;
    session.close().await.map_err(err("Session.Close"))?;
    result
}

fn err(call: &'static str) -> impl Fn(ashpd::Error) -> String {
    move |e| format!("{call}: {e}")
}

async fn barriers(portal: &InputCapture, session: &Session<InputCapture>) -> Result<(), String> {
    let zones: Zones = portal
        .zones(session, GetZonesOptions::default())
        .await
        .map_err(err("GetZones"))?
        .response()
        .map_err(err("GetZones response"))?;
    let list: Vec<Barrier> = zones.regions().iter().zip(1u32..).map(top_edge).collect();
    line(&format!("zones {:?}, barriers {list:?}", zones.regions()));
    let response = portal
        .set_pointer_barriers(
            session,
            &list,
            zones.zone_set(),
            SetPointerBarriersOptions::default(),
        )
        .await
        .map_err(err("SetPointerBarriers"))?
        .response()
        .map_err(err("SetPointerBarriers response"))?;
    line(&format!("failed barriers {:?}", response.failed_barriers()));
    Ok(())
}

/// A barrier along the top edge of `region`.
fn top_edge((region, id): (&ashpd::desktop::input_capture::Region, u32)) -> Barrier {
    let (x, y) = (region.x_offset(), region.y_offset());
    let right = x
        .saturating_add(i32::try_from(region.width()).unwrap_or(i32::MAX))
        .saturating_sub(1);
    let id = NonZero::new(id).unwrap_or(NonZero::<u32>::MIN);
    Barrier::new(id, (x, y, right, y))
}

/// The receiving end of the capture's EIS connection.
struct Receiver {
    connection: reis::event::Connection,
    events: reis::tokio::EiConvertEventStream,
}

/// Counts what arrives, and hands input back [`HAND_BACK`] after each
/// activation, until `duration` ends.
async fn watch(
    portal: &InputCapture,
    session: &Session<InputCapture>,
    mut receiver: Receiver,
    duration: Duration,
) -> Result<(), String> {
    let mut activated = portal.receive_activated().await.map_err(err("Activated"))?;
    let mut deactivated = portal
        .receive_deactivated()
        .await
        .map_err(err("Deactivated"))?;
    let mut disabled = portal.receive_disabled().await.map_err(err("Disabled"))?;
    let mut counts = BTreeMap::<&'static str, u64>::new();
    let end = tokio::time::sleep(duration);
    let release = tokio::time::sleep(Duration::MAX / 4);
    tokio::pin!(end, release);
    loop {
        tokio::select! {
            () = &mut end => break,
            () = &mut release => {
                line(&format!("received so far: {counts:?}"));
                hand_back(portal, session).await;
                release.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(86_400));
            }
            Some(a) = activated.next() => {
                line(&format!("activated: barrier {:?}", a.barrier_id()));
                release.as_mut().reset(tokio::time::Instant::now() + HAND_BACK);
            }
            Some(_) = deactivated.next() => line("deactivated"),
            Some(_) = disabled.next() => line("disabled"),
            Some(event) = receiver.events.next() => {
                count(&mut counts, event);
                // The seat bind is only buffered until flushed.
                if let Err(err) = receiver.connection.flush() {
                    line(&format!("ei: flush failed: {err}"));
                }
            }
        }
    }
    line(&format!("received events: {counts:?}"));
    Ok(())
}

async fn hand_back(portal: &InputCapture, session: &Session<InputCapture>) {
    let result = portal.release(session, ReleaseOptions::default()).await;
    line(&format!("released capture: {result:?}"));
}

fn count(counts: &mut BTreeMap<&'static str, u64>, event: Result<EiEvent, reis::Error>) {
    let kind = match event {
        Ok(EiEvent::SeatAdded(added)) => {
            added.seat.bind_capabilities(
                DeviceCapability::Pointer | DeviceCapability::Keyboard | DeviceCapability::Button,
            );
            "seat"
        }
        Ok(EiEvent::KeyboardKey(_)) => "key",
        Ok(EiEvent::Button(_)) => "button",
        Ok(EiEvent::PointerMotion(_)) => "motion",
        Ok(EiEvent::DeviceAdded(_)) => "device",
        Ok(EiEvent::DeviceStartEmulating(_)) => "start emulating",
        Ok(EiEvent::Frame(_)) => "frame",
        Ok(_) => "other",
        Err(_) => "error",
    };
    if counts.get(kind).is_none() {
        line(&format!("first {kind} event"));
    }
    *counts.entry(kind).or_insert(0) += 1;
}
