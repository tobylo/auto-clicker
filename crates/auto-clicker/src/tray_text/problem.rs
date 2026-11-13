//! What is wrong now, in Swedish, and the reasons notices share with it.
//! Quoted text stays as it is.

use auto_clicker_core::controller::{Blocked, NoRecording};
use auto_clicker_core::hotkeys::{Refusal, Unavailable};
use auto_clicker_core::message::{
    ConfigProblem, InputFailure, NoPointer, NoPositions, NoScreens, Problem,
};

use super::lines::key_name;

pub(super) fn swedish(problem: &Problem) -> String {
    match problem {
        Problem::Config(problem) => config(problem),
        Problem::Hotkeys(why) => unavailable(why),
        Problem::Blocked(why) => format!("Kan inte börja klicka: {}", blocked(why)),
        Problem::Input(why) => format!("Kan inte börja klicka: {}", input(why)),
        Problem::Unlisted(why) => format!("Inspelningarna kan inte listas: {why}"),
        Problem::CurrentUnread(why) => format!(
            "Inspelningen som snabbtangenten för uppspelning spelar kunde inte läsas, så den \
             spelar den nyaste: {why}"
        ),
        Problem::CurrentUnkept(why) => format!(
            "Inspelningen som snabbtangenten för uppspelning spelar kunde inte sparas till \
             nästa start: {why}"
        ),
        Problem::NoPositions(why) => {
            format!("Inspelningar sparar bara rörelser: {}", no_positions(*why))
        }
    }
}

fn no_positions(why: NoPositions) -> &'static str {
    match why {
        NoPositions::Screens(why) => no_screens(why),
        NoPositions::Pointer(why) => no_pointer(why),
    }
}

/// Why the screen layout is not known, in Swedish.
pub(super) fn no_screens(why: NoScreens) -> &'static str {
    match why {
        NoScreens::NoDisplay => {
            "programmet startade utan en Wayland-display (WAYLAND_DISPLAY); se README"
        }
        NoScreens::CannotConnect => "programmet kunde inte ansluta till Wayland-displayen",
        NoScreens::NoXdgOutput => "skrivbordet berättar inte var skärmarna är (ingen xdg-output)",
        NoScreens::Lost => "anslutningen till Wayland-displayen bröts",
        NoScreens::ProtocolError => "Wayland-displayen avvisade programmets förfrågningar",
        NoScreens::NotWhole => "skrivbordet har inte beskrivit alla skärmar",
    }
}

fn no_pointer(why: NoPointer) -> &'static str {
    match why {
        NoPointer::NoSource => "inget på det här skrivbordet rapporterar var pekaren är",
        NoPointer::Unusable => "KWins pekarskript kan inte användas; se loggen",
    }
}

fn config(problem: &ConfigProblem) -> String {
    match problem {
        ConfigProblem::NoDirectory => {
            "ingen konfigurationsmapp: varken XDG_CONFIG_HOME eller HOME är satt".to_owned()
        }
        ConfigProblem::Unreadable { file, why } => format!("{file}: filen kunde inte läsas: {why}"),
        ConfigProblem::Invalid {
            file,
            line: Some(line),
            why,
        } => format!("{file}, rad {line}: {why}"),
        ConfigProblem::Invalid {
            file,
            line: None,
            why,
        } => format!("{file} gick inte att använda: {why}"),
    }
}

pub(super) fn blocked(why: &Blocked) -> String {
    match why {
        Blocked::Locked => "skärmen är låst".to_owned(),
        Blocked::Inactive => "en annan session är framme".to_owned(),
        Blocked::Sleeping => "datorn ska gå i viloläge".to_owned(),
        Blocked::Unknown(why) => format!("sessionens läge är okänt: {why}"),
    }
}

pub(super) fn input(why: &InputFailure) -> String {
    match why {
        InputFailure::NoPermission(e) => format!(
            "ingen behörighet att öppna /dev/uinput ({e}); installera paketets udev-regel och \
             logga in igen, eller lägg till din användare i gruppen `input`"
        ),
        InputFailure::UinputMissing(e) => {
            format!("/dev/uinput finns inte ({e}); ladda kärnmodulen med `sudo modprobe uinput`")
        }
        InputFailure::CreateDevice(e) => {
            format!("kunde inte skapa en virtuell inmatningsenhet: {e}")
        }
        InputFailure::Write(e) => {
            format!("kunde inte skriva till en virtuell inmatningsenhet: {e}")
        }
        InputFailure::PortalUnavailable(why) => {
            format!("skrivbordets portal för fjärrinmatning kan inte användas: {why}")
        }
        InputFailure::NotAllowed => {
            "inte tillåtet i skrivbordets dialog; nästa start frågar igen".to_owned()
        }
        InputFailure::AllowedNow => "tillåtet; starta igen".to_owned(),
        InputFailure::Portal(why) => format!("portalen för fjärrinmatning misslyckades: {why}"),
        InputFailure::SessionLost(why) => {
            format!("skrivbordet avslutade fjärrinmatningen: {why}")
        }
    }
}

pub(super) fn unavailable(why: &Unavailable) -> String {
    const OFF: &str = "snabbtangenterna är inte tillgängliga";
    match why {
        Unavailable::NoKeyboard => format!("{OFF}: inget tangentbord hittades"),
        Unavailable::NoPermission => format!(
            "{OFF}: tangentborden kan inte läsas; lägg till din användare i gruppen input och \
             logga in igen, eller sätt hotkeys = \"portal\" under [input] för skrivbordets \
             genvägar"
        ),
        Unavailable::ReadFailed(error) => format!("{OFF}: ett tangentbord kan inte läsas: {error}"),
        Unavailable::WatchFailed(error) => format!(
            "snabbtangenterna stoppade: bevakningen av tangentborden misslyckades: {error}; \
             starta om programmet"
        ),
        Unavailable::Desktop(why) => format!("{OFF}: {why}"),
    }
}

/// Why no recording starts, after "Spelar inte in: ".
pub(super) fn no_recording(why: &NoRecording) -> String {
    match why {
        NoRecording::DesktopHotkeys => "inspelning går inte med skrivbordets genvägar".to_owned(),
        NoRecording::Keyboards(why) => unavailable(why),
    }
}

/// Why the program may not press a key the desktop's hotkeys use.
pub(super) fn own_key(why: &Refusal) -> String {
    match why {
        Refusal::Modifier(key) => format!(
            "{} är en modifierare, och om programmet håller den ändras vilken snabbtangent \
             skrivbordet ser",
            key_name(*key)
        ),
        Refusal::HotkeyKey { key, trigger } => format!(
            "{} är tangenten i snabbtangenten {trigger}, och skrivbordet kan inte skilja \
             programmets tangenttryck från dina",
            key_name(*key)
        ),
        Refusal::Unreadable(trigger) => format!(
            "snabbtangenten \u{201d}{trigger}\u{201d} ligger inte på en funktionstangent (F1 \
             till F24), så programmet kan inte avgöra om dess egna tangenttryck skulle utlösa \
             den; ändra den i Systeminställningar"
        ),
        Refusal::NotReported => "skrivbordet har inte sagt än vilka tangenter snabbtangenterna \
                                 använder, och programmet får inte trycka på någon av dem"
            .to_owned(),
    }
}
