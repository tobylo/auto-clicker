//! The tray's notice in Swedish. Quoted text stays as it is.

use auto_clicker_core::message::{
    Hindrance, Notice, PlayStart, PointScreens, SaveFailure, Saving, Start, Stop, Unplayable,
};
use auto_clicker_core::playback::Fallback;

use super::lines::recording_name;
use super::problem::{blocked, input, no_recording, own_key, unavailable};

pub(super) fn swedish(notice: &Notice) -> String {
    match notice {
        Notice::Refused { start, why } => format!("{}: {}", refused(*start), hindrance(why)),
        Notice::Unplayable { recording, why } => format!(
            "Spelar inte upp {}: {}",
            recording_name(recording),
            unplayable(why)
        ),
        Notice::NothingToPlay => "Inget att spela upp: inga inspelningar än".to_owned(),
        Notice::Stopped(why) => stop(why),
        Notice::RunFailed { playback, why } => {
            let what = if *playback {
                "Uppspelningen"
            } else {
                "Klickandet"
            };
            format!("{what} stoppades: {}", input(why))
        }
        Notice::Finished(Some(recording)) => {
            format!("Spelade upp {} till slutet", recording_name(recording))
        }
        Notice::Finished(None) => "Gjorde alla åtgärder".to_owned(),
        Notice::Playing(start) => play_start(*start),
        Notice::Recorded {
            stop: Some(why),
            saving: saved,
        } => format!("{}. {}", stop(why), saving(saved)),
        Notice::Recorded {
            stop: None,
            saving: saved,
        } => saving(saved),
    }
}

fn refused(start: Start) -> &'static str {
    match start {
        Start::Clicking => "Startade inte",
        Start::Playback => "Spelar inte upp",
        Start::Recording => "Spelar inte in",
    }
}

fn hindrance(why: &Hindrance) -> String {
    match why {
        Hindrance::Blocked(why) => blocked(why),
        Hindrance::OwnKey(why) => own_key(why),
        Hindrance::Input(why) => input(why),
        Hindrance::HeldTooLong { seconds } => {
            format!("snabbtangenten hölls nere i {seconds} s; släpp den för att starta")
        }
        Hindrance::ReleaseLost(hotkey) => {
            format!("kunde inte avgöra när snabbtangenten släpptes; tryck {hotkey} igen")
        }
        Hindrance::HotkeysStopped => "snabbtangenterna slutade fungera".to_owned(),
        Hindrance::ClickingOn => "klickandet är på; stoppa det först".to_owned(),
        Hindrance::Unlisted(why) => format!("inspelningarna kan inte listas: {why}"),
        Hindrance::NoRecording(why) => no_recording(why),
        Hindrance::NoClock => "klockan kunde inte läsas".to_owned(),
        Hindrance::Hotkeys(why) => unavailable(why),
        Hindrance::Screens(why) => point_screens(*why).to_owned(),
    }
}

fn point_screens(why: PointScreens) -> &'static str {
    match why {
        PointScreens::Changed => {
            "skärmarna har ändrats sedan den fasta punkten valdes: välj den igen i Inställningar"
        }
        PointScreens::Unknown => {
            "skärmlayouten är inte känd, så den valda punkten kan inte kontrolleras"
        }
    }
}

fn unplayable(why: &Unplayable) -> String {
    match why {
        Unplayable::Missing => "filen finns inte längre".to_owned(),
        Unplayable::Unreadable(why) => format!("filen kunde inte läsas: {why}"),
        Unplayable::TooLarge { mib } => format!("filen är större än {mib} MiB"),
        Unplayable::Invalid(why) => format!("filen är ingen giltig inspelning: {why}"),
        Unplayable::TooShort { ms } => {
            format!("den är kortare än {ms} ms, för kort för att spelas mer än en gång")
        }
        Unplayable::PressesHotkeyKey(why) => {
            format!("den trycker på en snabbtangents tangent: {}", own_key(why))
        }
    }
}

fn stop(why: &Stop) -> String {
    match why {
        Stop::Blocked(why) => format!("Stoppat: {}", blocked(why)),
        Stop::OwnKey(why) => format!("Stoppat: {}", own_key(why)),
        Stop::LayoutChanged => "Uppspelningen stoppades: skärmlayouten ändrades".to_owned(),
        Stop::LayoutUnknown => {
            "Uppspelningen stoppades: skärmlayouten är inte längre känd".to_owned()
        }
        Stop::Screens(why) => format!("Klickandet stoppades: {}", point_screens(*why)),
        Stop::Hotkeys(why) => format!("Inspelningen stoppades: {}", unavailable(why)),
        Stop::Full => {
            "Inspelningen stoppades: den nådde den största storlek en inspelning får ha".to_owned()
        }
        Stop::Limit { minutes } => {
            format!("Inspelningen stoppades vid sin gräns på {minutes} minuter")
        }
    }
}

fn play_start(start: PlayStart) -> String {
    let held = |seconds: u64| {
        format!(
            "Spelar upp fast snabbtangenten fortfarande hölls nere efter {seconds} s; dess \
             tangenter blandas in i det som spelas upp"
        )
    };
    match start {
        PlayStart::Held { seconds } => held(seconds),
        PlayStart::Relative(why) => fallback(why).to_owned(),
        PlayStart::HeldAndRelative {
            seconds,
            fallback: why,
        } => {
            format!("{}. {}", held(seconds), fallback(why))
        }
    }
}

fn fallback(why: Fallback) -> &'static str {
    match why {
        Fallback::Unknown => {
            "Spelar upp med relativa rörelser: skärmlayouten är inte känd, så de inspelade \
             positionerna kan inte användas"
        }
        Fallback::Changed => {
            "Spelar upp med relativa rörelser: en skärm som inspelningen använde har flyttats, \
             bytt storlek eller försvunnit"
        }
    }
}

fn saving(saving: &Saving) -> String {
    match saving {
        Saving::Nothing => "Inget spelades in, så inget sparades".to_owned(),
        Saving::Saved { name, .. } => {
            // A new recording's generated name is said as the menu says it.
            let shown = match saving.titled() {
                Some(titled) if titled.generated() => recording_name(&titled),
                _ => name.to_string(),
            };
            format!("Sparade \u{201d}{shown}\u{201d}")
        }
        Saving::Failed(why) => format!("Inspelningen kunde inte sparas: {}", save_failure(why)),
    }
}

fn save_failure(why: &SaveFailure) -> String {
    match why {
        SaveFailure::NoDataDirectory => {
            "ingen datamapp: varken XDG_DATA_HOME eller HOME är satt".to_owned()
        }
        SaveFailure::Invalid(why) => format!("det inspelade är ingen giltig inspelning: {why}"),
        SaveFailure::Write(why) => why.to_string(),
    }
}
