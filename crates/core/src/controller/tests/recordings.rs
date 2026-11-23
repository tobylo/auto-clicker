//! The recordings manager: what the settings window does with the
//! recordings (rename, delete, choose, play), the current recording kept
//! across restarts, and the lists the tray and the window show.

use std::fs;
use std::num::NonZeroU32;
use std::path::PathBuf;

use super::recording::KEYBOARD;
use super::settings::Window;
use super::*;
use crate::config::CountDraft;
use crate::message::ChangeFailure;
use crate::playback::{PlaybackSettings, Speed};
use crate::recording::{Change, CurrentFile, FileName, load, recordable};
use crate::window::{
    Ask, Deletion, Files, NotDeleted, Outcome, RecordingAction, RecordingOutcome, Removal,
};

const W: u16 = 17;
const PLAY: HotkeyEvent = HotkeyEvent::Fired(Action::TogglePlayback);

/// A config that plays once, with a tray countdown of `delay` seconds.
fn config(delay: u64) -> Config {
    Config {
        tray_start_delay: secs(delay),
        playback: PlaybackSettings {
            repeat: Repeat::Times(NonZeroU32::MIN),
            ..PlaybackSettings::default()
        },
        ..Config::default()
    }
}

/// W held from 10 ms to 500 ms in a recording of a second, made at `time`
/// ("18:00:00") on 2026-11-09.
fn store(shared: &Shared, time: &str) -> FileName {
    let held = recordable(W).unwrap();
    shared.store(
        time,
        1000,
        &[(10, Change::Press(held)), (500, Change::Release(held))],
    )
}

/// A recordings folder holding a recording made at each of `times`.
fn shared_with(times: &[&str]) -> (Shared, Vec<FileName>) {
    let folder = Arc::new(TempDir::new());
    let shared = shared_over(&folder);
    let files = times.iter().map(|time| store(&shared, time)).collect();
    (shared, files)
}

/// A fresh platform over the recordings folder `folder`.
fn shared_over(folder: &Arc<TempDir>) -> Shared {
    let shared = Shared::default();
    shared.with(|r| r.recordings = Some(Arc::clone(folder)));
    shared
}

fn folder(shared: &Shared) -> PathBuf {
    shared.with(|r| r.recordings.as_ref().unwrap().path().to_path_buf())
}

/// Where a daemon keeps its current recording, in `dir`.
fn state_file(dir: &TempDir) -> PathBuf {
    dir.path().join("auto-clicker").join("current-recording")
}

/// A daemon over `shared` that keeps its current recording at `current`,
/// with a settings window open when `window` says so.
struct Setup {
    d: Daemon,
    window: Option<Window>,
    opener: WindowOpener,
    current: PathBuf,
}

async fn setup_at(shared: Shared, config: Config, current: PathBuf, window: bool) -> Setup {
    let mut opener = None;
    let d = daemon_prepared(config, shared, true, |controller| {
        controller.keep_current_file(CurrentFile::at(current.clone()));
        opener = Some(controller.window_opener());
    })
    .await;
    let opener = opener.unwrap();
    let window = window.then(|| Window::open(&opener));
    d.settle().await;
    Setup {
        d,
        window,
        opener,
        current,
    }
}

/// [`setup_at`] a state file of its own, which `dir` holds.
async fn setup(shared: Shared, config: Config, dir: &TempDir, window: bool) -> Setup {
    setup_at(shared, config, state_file(dir), window).await
}

impl Setup {
    fn window(&mut self) -> &mut Window {
        self.window.as_mut().expect("a window")
    }

    async fn open_window(&mut self) {
        self.window = Some(Window::open(&self.opener));
        self.d.settle().await;
    }

    /// Asks the window's `action` with `file`, and returns what came of it.
    async fn ask(&mut self, file: &FileName, action: RecordingAction) -> Outcome {
        let ask = Ask::Recording {
            file: file.clone(),
            action,
        };
        let Setup { d, window, .. } = self;
        window.as_mut().expect("a window").ask(d, ask).await
    }

    /// Asks the window to delete `which`, and returns what came of it.
    async fn delete(&mut self, which: Deletion) -> Outcome {
        let Setup { d, window, .. } = self;
        window
            .as_mut()
            .expect("a window")
            .ask(d, Ask::Delete(which))
            .await
    }

    /// The file name the state file holds, as written.
    fn kept(&self) -> String {
        fs::read_to_string(&self.current).unwrap_or_default()
    }

    /// The tray's labels.
    fn tray(&self) -> Vec<String> {
        let view = self.d.view();
        view.recordings.iter().map(|l| l.label.clone()).collect()
    }

    /// Plays the current recording with the hotkey; what plays.
    async fn hotkey_plays(&self) -> Option<String> {
        self.d.hotkey(PLAY).await;
        self.d.hotkey(HotkeyEvent::ModifiersReleased).await;
        let playing = self.d.view().playing.map(|playing| playing.to_string());
        sleep(secs(2)).await;
        self.d.settle().await;
        assert_eq!(self.d.activity(), Activity::Idle);
        playing
    }
}

fn done(outcome: RecordingOutcome) -> Outcome {
    Outcome::Recording(outcome)
}

/// What a change or a play that did not happen says, in English.
fn not_done(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Recording(
            said @ (RecordingOutcome::NotRenamed(..) | RecordingOutcome::NotPlaying(_)),
        ) => said.to_string(),
        other => panic!("{other:?}"),
    }
}

/// A delete of `files`.
fn these(files: &[&FileName]) -> Deletion {
    Deletion::Files(Files::new(files.iter().map(|&file| file.clone()).collect()).unwrap())
}

/// What a delete that removed `n` and failed nothing answers.
fn deleted(n: u64) -> Outcome {
    Outcome::Deleted(Removal {
        deleted: n,
        ..Removal::default()
    })
}

/// What a delete that went ahead says, in English.
fn removal_said(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Deleted(removal) => removal.to_string(),
        other => panic!("{other:?}"),
    }
}

fn label(time: &str) -> String {
    format!("Recording 2026-11-09 {time}")
}

// Renaming

#[tokio::test(start_paused = true)]
async fn a_rename_rewrites_the_name_in_the_same_file_and_the_tray_shows_it() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00"]);
        let file = &files[0];
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        let rename = |name: &str| RecordingAction::Rename {
            name: name.to_owned(),
        };

        let outcome = s.ask(file, rename("Farm loop")).await;

        assert_eq!(outcome, done(RecordingOutcome::Renamed));
        let dir = folder(&s.d.shared);
        assert_eq!(load(&dir, file).unwrap().name(), "Farm loop");
        let entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries, [file.to_string()]);
        let titled = "Farm loop (2026-11-09 18:00:00)";
        assert_eq!(s.tray(), [titled]);
        let listed = s.window().view().recordings.listed;
        assert_eq!(listed[0].name.as_deref(), Some("Farm loop"));
        // What plays is named as the lists name it.
        assert_eq!(s.hotkey_plays().await.as_deref(), Some(titled));

        // A name a recording may not have: refused, the file as it was.
        let before = fs::read(dir.join(file.as_str())).unwrap();
        let outcome = s.ask(file, rename(" ")).await;
        assert_eq!(not_done(&outcome), "Not renamed: the recording has no name");
        assert_eq!(fs::read(dir.join(file.as_str())).unwrap(), before);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_rename_of_a_file_gone_or_damaged_says_so_and_lists_again() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        let dir = folder(&s.d.shared);
        fs::remove_file(dir.join(files[0].as_str())).unwrap();
        fs::write(dir.join(files[1].as_str()), b"{}").unwrap();
        let rename = RecordingAction::Rename {
            name: "New".to_owned(),
        };

        let gone = s.ask(&files[0], rename.clone()).await;
        assert_eq!(not_done(&gone), "Not renamed: the file is no longer there");
        assert_eq!(s.tray(), [label("18:01:00")]);
        let damaged = s.ask(&files[1], rename).await;
        assert!(
            not_done(&damaged).starts_with("Not renamed: "),
            "{damaged:?}"
        );
        assert_eq!(fs::read(dir.join(files[1].as_str())).unwrap(), b"{}");
    })
    .await;
}

// Deleting

#[tokio::test(start_paused = true)]
async fn a_delete_drops_the_recording_and_the_current_one_falls_back_to_the_newest() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00", "18:02:00"]);
        let [oldest, middle, newest] = [&files[0], &files[1], &files[2]];
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        assert_eq!(
            s.ask(middle, RecordingAction::Choose).await,
            done(RecordingOutcome::Chosen)
        );
        assert_eq!(s.d.view().current.as_ref(), Some(middle));

        assert_eq!(s.delete(these(&[oldest])).await, deleted(1));
        assert_eq!(s.tray(), [label("18:02:00"), label("18:01:00")]);
        assert_eq!(s.d.view().current.as_ref(), Some(middle));

        assert_eq!(s.delete(these(&[middle])).await, deleted(1));
        let gone = !folder(&s.d.shared).join(middle.as_str()).exists();
        let window = s.window().view().recordings;
        let current = (window.current, s.d.view().current);
        assert_eq!(
            (gone, current),
            (true, (Some(newest.clone()), Some(newest.clone())))
        );
        assert_eq!(s.hotkey_plays().await, Some(label("18:02:00")));

        // Gone already: said so, not done twice.
        let again = s.delete(these(&[middle])).await;
        assert_eq!(
            removal_said(&again),
            "Not deleted: Recording 2026-11-09 18:01:00: the file is no longer there."
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_delete_of_several_takes_them_together_and_the_current_one_with_them() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00", "18:02:00", "18:03:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        let chosen = s.ask(&files[2], RecordingAction::Choose).await;
        assert_eq!(chosen, done(RecordingOutcome::Chosen));

        let outcome = s.delete(these(&[&files[0], &files[2], &files[3]])).await;

        assert_eq!(outcome, deleted(3));
        let window = s.window().view().recordings;
        let left: Vec<&FileName> = window.listed.iter().map(|listed| &listed.file).collect();
        assert_eq!(left, [&files[1]]);
        // The chosen one went, so the one left plays.
        assert_eq!(
            (window.current.as_ref(), s.tray()),
            (Some(&files[1]), vec![label("18:01:00")])
        );
        assert_eq!(s.hotkey_plays().await, Some(label("18:01:00")));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_failure_partway_names_the_file_and_the_rest_are_still_deleted() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00", "18:02:00"]);
        let dir = folder(&shared);
        // A folder with a recording's name, which removing a file cannot
        // remove.
        let stuck = FileName::parse("2026-11-09_18-01-30.000.json").unwrap();
        fs::create_dir(dir.join(stuck.as_str())).unwrap();
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        let listed = s.window().view().recordings.listed.len();
        assert_eq!(listed, 4);

        let outcome = s.delete(these(&[&files[0], &stuck, &files[2]])).await;

        let Outcome::Deleted(removal) = &outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(removal.deleted, 2);
        let [NotDeleted { recording, why }] = &removal.failed[..] else {
            panic!("{removal:?}");
        };
        assert_eq!(recording.file, stuck);
        assert!(matches!(why, ChangeFailure::NotDone { .. }), "{why:?}");
        assert!(removal.fell_short());
        // The list shows what is left.
        let left: Vec<FileName> = s
            .window()
            .view()
            .recordings
            .listed
            .into_iter()
            .map(|listed| listed.file)
            .collect();
        assert_eq!(left, [stuck.clone(), files[1].clone()]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn delete_all_takes_every_recording_the_unlisted_ones_too() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00"]);
        let dir = folder(&shared);
        for n in 0..WINDOW_RECORDINGS + 5 {
            let name = format!("2026-11-09_17-{:02}-{:02}.000.json", n / 60, n % 60);
            fs::write(dir.join(name), b"").unwrap();
        }
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        let chosen = s.ask(&files[0], RecordingAction::Choose).await;
        assert_eq!(chosen, done(RecordingOutcome::Chosen));
        let view = s.window().view().recordings;
        assert_eq!((view.listed.len(), view.more), (WINDOW_RECORDINGS, 6));

        let outcome = s.delete(Deletion::All).await;

        let all = u64::try_from(WINDOW_RECORDINGS + 6).unwrap();
        assert_eq!(outcome, deleted(all));
        let view = s.window().view().recordings;
        assert_eq!((view.listed.len(), view.more, view.current), (0, 0, None));
        assert!(s.tray().is_empty());
        let entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries, ["notes.txt"]);
        // Nothing left: the hotkey has nothing to play.
        assert_eq!(s.hotkey_plays().await, None);
        // A second asks finds none.
        assert_eq!(s.delete(Deletion::All).await, deleted(0));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_delete_with_no_recordings_folder_says_why_and_deletes_nothing() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        fs::remove_file(folder(&s.d.shared).join(files[0].as_str())).unwrap();
        fs::remove_dir(folder(&s.d.shared)).unwrap();

        let outcome = s.delete(Deletion::All).await;

        assert_eq!(outcome, Outcome::NotDeleted(ChangeFailure::Missing));
    })
    .await;
}

// The current recording

#[tokio::test(start_paused = true)]
async fn the_current_recording_survives_a_restart_and_a_bad_state_file_means_the_newest() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let recordings = shared.with(|r| r.recordings.clone()).unwrap();
        let state = TempDir::new();
        // No file: the newest, and nothing wrong.
        let mut s = setup(shared, config(0), &state, true).await;
        assert_eq!(s.d.view().current.as_ref(), Some(&files[1]));
        assert!(s.d.problems().is_empty());
        s.ask(&files[0], RecordingAction::Choose).await;
        drop(s);

        // Started again: what was chosen.
        let s = setup(shared_over(&recordings), config(0), &state, false).await;
        assert_eq!(s.d.view().current.as_ref(), Some(&files[0]));
        assert_eq!(s.hotkey_plays().await, Some(label("18:00:00")));
        assert!(s.d.problems().is_empty());
        drop(s);

        // A file that holds no file name: the newest, and why.
        fs::write(state_file(&state), b"../config.toml\n").unwrap();
        let s = setup(shared_over(&recordings), config(0), &state, false).await;
        let problems = s.d.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with("The recording the playback hotkey plays could not be read"),
            "{problems:?}"
        );
        assert_eq!(s.hotkey_plays().await, Some(label("18:01:00")));
        // Played, it is kept, and the problem is over.
        assert_eq!(s.kept(), format!("{}\n", files[1]));
        assert!(s.d.problems().is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_current_recording_that_is_gone_marks_the_newest_and_one_older_than_the_list_marks_none()
{
    local(async {
        let times: Vec<String> = (0..12).map(|minute| format!("17:{minute:02}:00")).collect();
        let times: Vec<&str> = times.iter().map(String::as_str).collect();
        let (shared, files) = shared_with(&times);
        let recordings = shared.with(|r| r.recordings.clone()).unwrap();
        let state = TempDir::new();
        fs::create_dir_all(state_file(&state).parent().unwrap()).unwrap();

        // The oldest, left out of the tray's ten: it plays, none is marked.
        fs::write(state_file(&state), format!("{}\n", files[0])).unwrap();
        let s = setup(shared, config(0), &state, false).await;
        assert_eq!(s.d.view().current, None);
        assert_eq!(s.hotkey_plays().await, Some(label("17:00:00")));
        drop(s);

        // Gone, with older ones still left out of the list: the newest is
        // marked, and plays.
        let oldest = recordings.path().join(files[0].as_str());
        let bytes = fs::read(&oldest).unwrap();
        fs::remove_file(&oldest).unwrap();
        let s = setup(shared_over(&recordings), config(0), &state, false).await;
        assert_eq!(s.d.view().current.as_ref(), Some(&files[11]));

        // Back before the hotkey: it plays, and the newest is no longer
        // marked.
        fs::write(&oldest, &bytes).unwrap();
        assert_eq!(s.hotkey_plays().await, Some(label("17:00:00")));
        assert_eq!(s.d.view().current, None);
        drop(s);

        // Gone again: the newest plays.
        fs::remove_file(&oldest).unwrap();
        let s = setup(shared_over(&recordings), config(0), &state, false).await;
        assert_eq!(s.hotkey_plays().await, Some(label("17:11:00")));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_new_recording_a_play_from_the_tray_and_a_choice_in_the_window_make_it_current() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        assert_eq!(s.kept(), "");

        // From the tray.
        s.d.send(Message::Play(files[0].clone())).await;
        sleep(secs(2)).await;
        s.d.settle().await;
        assert_eq!(s.kept(), format!("{}\n", files[0]));
        assert_eq!(s.d.view().current.as_ref(), Some(&files[0]));

        // From the window.
        let outcome = s.ask(&files[1], RecordingAction::Choose).await;
        assert_eq!(outcome, done(RecordingOutcome::Chosen));
        assert_eq!(s.kept(), format!("{}\n", files[1]));
        assert_eq!(
            s.window().view().recordings.current.as_ref(),
            Some(&files[1])
        );

        // A new recording: made at 18:04:05.123 on the fake clock.
        s.d.record_hotkey().await;
        s.d.key(KEYBOARD, W, true);
        s.d.key(KEYBOARD, W, false);
        s.d.record_hotkey().await;
        let made = FileName::parse("2026-11-09_18-04-05.123.json").unwrap();
        assert_eq!(s.kept(), format!("{made}\n"));
        assert_eq!(s.d.view().current.as_ref(), Some(&made));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_choice_of_a_recording_gone_since_the_list_was_read_marks_the_newest() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        s.ask(&files[0], RecordingAction::Choose).await;
        fs::remove_file(folder(&s.d.shared).join(files[0].as_str())).unwrap();

        // Asked twice, as a window that had not seen it go would.
        for _ in 0..2 {
            s.ask(&files[0], RecordingAction::Choose).await;
            assert_eq!(s.d.view().current.as_ref(), Some(&files[1]));
            assert_eq!(s.tray(), [label("18:01:00")]);
        }
        assert_eq!(s.hotkey_plays().await, Some(label("18:01:00")));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_state_file_is_written_only_when_the_current_recording_changes() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        s.ask(&files[0], RecordingAction::Choose).await;
        // Changed behind the program's back: a write would undo it.
        fs::write(&s.current, format!("{}\n", files[1])).unwrap();

        s.ask(&files[0], RecordingAction::Choose).await;
        s.hotkey_plays().await;
        s.d.send(Message::Play(files[0].clone())).await;
        sleep(secs(2)).await;
        s.d.settle().await;

        assert_eq!(s.kept(), format!("{}\n", files[1]));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_current_recording_that_cannot_be_kept_is_used_and_shown_as_a_problem() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, true).await;
        // A folder where the file would go.
        fs::create_dir_all(&s.current).unwrap();

        let outcome = s.ask(&files[0], RecordingAction::Choose).await;

        assert_eq!(outcome, done(RecordingOutcome::Chosen));
        let problems = s.d.problems();
        assert!(
            problems.iter().any(|p| p.starts_with(
                "The recording the playback hotkey plays could not be kept for the next start"
            )),
            "{problems:?}"
        );
        assert_eq!(s.hotkey_plays().await, Some(label("18:00:00")));

        // Once it can be written, choosing the same one again keeps it.
        fs::remove_dir(&s.current).unwrap();
        s.ask(&files[0], RecordingAction::Choose).await;
        assert_eq!(s.kept(), format!("{}\n", files[0]));
        assert!(s.d.problems().is_empty());
    })
    .await;
}

// Busy

#[tokio::test(start_paused = true)]
async fn nothing_is_done_with_a_recording_while_anything_runs_or_is_about_to() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(5), &state, true).await;
        let dir = folder(&s.d.shared);
        let before: Vec<Vec<u8>> = files
            .iter()
            .map(|file| fs::read(dir.join(file.as_str())).unwrap())
            .collect();
        let actions = [
            RecordingAction::Rename {
                name: "New".to_owned(),
            },
            RecordingAction::Choose,
            RecordingAction::Play,
        ];
        let deletions = [these(&[&files[0], &files[1]]), Deletion::All];
        let starts = [
            Message::Start,
            Message::Record,
            Message::Play(files[1].clone()),
        ];
        for start in starts {
            s.d.send(start.clone()).await;
            assert_ne!(s.d.activity(), Activity::Idle, "{start:?}");
            for action in actions.clone() {
                let outcome = s.ask(&files[0], action.clone()).await;
                assert_eq!(outcome, Outcome::Busy, "{start:?} {action:?}");
            }
            for which in deletions.clone() {
                let outcome = s.delete(which.clone()).await;
                assert_eq!(outcome, Outcome::Busy, "{start:?} {which:?}");
            }
            let stop = match start {
                Message::Record => Message::StopRecording,
                _ => Message::Stop,
            };
            s.d.send(stop).await;
            assert_eq!(s.d.activity(), Activity::Idle);
        }
        // Playing.
        s.d.hotkey(PLAY).await;
        s.d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(s.d.activity(), Activity::Playing);
        for action in actions {
            assert_eq!(
                s.ask(&files[0], action.clone()).await,
                Outcome::Busy,
                "{action:?}"
            );
        }
        // The playing recording, and every other, stays.
        for which in deletions {
            assert_eq!(s.delete(which.clone()).await, Outcome::Busy, "{which:?}");
        }
        let after: Vec<Vec<u8>> = files
            .iter()
            .map(|file| fs::read(dir.join(file.as_str())).unwrap())
            .collect();
        assert_eq!(after, before);
        // What the tray played was kept; the choice asked while busy was
        // not.
        assert_eq!(s.kept(), format!("{}\n", files[1]));
    })
    .await;
}

// Playing from the window

#[tokio::test(start_paused = true)]
async fn play_in_the_window_counts_down_like_the_tray_and_says_why_when_it_cannot() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00", "18:01:00"]);
        let state = TempDir::new();
        let mut s = setup(shared, config(3), &state, true).await;

        let outcome = s.ask(&files[0], RecordingAction::Play).await;

        assert_eq!(outcome, done(RecordingOutcome::Playing));
        assert_eq!(s.d.activity(), Activity::PlayingIn(3));
        assert_eq!(
            s.d.view().playing.map(|playing| playing.to_string()),
            Some(label("18:00:00"))
        );
        s.d.send(Message::Stop).await;

        fs::write(folder(&s.d.shared).join(files[1].as_str()), b"").unwrap();
        let outcome = s.ask(&files[1], RecordingAction::Play).await;
        assert!(
            not_done(&outcome).starts_with(&format!("Not playing {}: ", label("18:01:00"))),
            "{outcome:?}"
        );
        assert_eq!(s.d.activity(), Activity::Idle);
    })
    .await;
}

// The lists

#[tokio::test(start_paused = true)]
async fn the_window_lists_two_hundred_and_how_many_more_while_the_tray_keeps_ten() {
    local(async {
        let (shared, _) = shared_with(&[]);
        let dir = folder(&shared);
        for n in 0..205 {
            let name = format!("2026-11-09_17-{:02}-{:02}.000.json", n / 60, n % 60);
            fs::write(dir.join(name), b"").unwrap();
        }
        let state = TempDir::new();
        let mut s = setup(shared, config(0), &state, false).await;
        assert_eq!(s.d.view().recordings.len(), LISTED_RECORDINGS);

        s.open_window().await;

        let view = s.window().view().recordings;
        assert_eq!((view.listed.len(), view.more), (WINDOW_RECORDINGS, 5));
        assert_eq!(view.listed[0].file.as_str(), "2026-11-09_17-03-24.000.json");
        // A file whose start is no recording's has no name to show.
        assert_eq!(view.listed[0].name, None);
        // The tray's view holds the same list; its menu shows the first
        // ten.
        assert_eq!(s.d.view().recordings.len(), WINDOW_RECORDINGS);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_window_opened_while_something_runs_gets_its_list_once_idle() {
    local(async {
        let (shared, _) = shared_with(&[]);
        let dir = folder(&shared);
        for n in 0..12 {
            fs::write(dir.join(format!("2026-11-09_17-00-{n:02}.000.json")), b"").unwrap();
        }
        let state = TempDir::new();
        let mut s = setup(shared, config(5), &state, false).await;
        s.d.send(Message::Start).await;

        s.open_window().await;
        assert_eq!(s.window().view().recordings.listed.len(), LISTED_RECORDINGS);

        s.d.send(Message::Stop).await;
        let view = s.window().view().recordings;
        assert_eq!((view.listed.len(), view.more), (12, 0));
    })
    .await;
}

// Loop and speed

#[tokio::test(start_paused = true)]
async fn loop_and_speed_round_trip_through_the_config_file() {
    local(async {
        let mut s = super::settings::setup().await;
        let speed_and_loop = super::settings::draft(|d| {
            d.playback.speed = 1.5;
            d.playback.repeat = CountDraft::UntilStopped;
        });

        assert_eq!(
            s.window.save(&s.d, speed_and_loop.clone()).await,
            Outcome::Saved
        );

        let written = Config::parse(&fs::read_to_string(&s.path).unwrap()).unwrap();
        let expected = PlaybackSettings {
            speed: Speed::from_ratio(1.5).unwrap(),
            repeat: Repeat::UntilStopped,
        };
        assert_eq!(written.playback, expected);
        assert_eq!(s.d.view().playback, expected);
        assert_eq!(s.window.view().settings.playback, speed_and_loop.playback);
    })
    .await;
}

/// A temporary file a save of `target` in `dir` by process `pid` left, last
/// written `ago`.
fn left_temp(dir: &std::path::Path, target: &str, pid: u32, ago: Duration) -> PathBuf {
    let path = dir.join(format!(".{target}.{pid}.0123456789abcdef.tmp"));
    fs::write(&path, b"half").unwrap();
    let then = std::time::SystemTime::now() - ago;
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(then)
        .unwrap();
    path
}

#[tokio::test(start_paused = true)]
async fn a_start_removes_what_saves_that_crashed_an_hour_ago_left() {
    local(async {
        let (shared, files) = shared_with(&["18:00:00"]);
        let recordings = folder(&shared);
        let state = TempDir::new();
        let state_dir = state_file(&state).parent().unwrap().to_path_buf();
        fs::create_dir_all(&state_dir).unwrap();
        let hours = |n: u64| Duration::from_secs(n * 60 * 60);
        let old = left_temp(&recordings, files[0].as_str(), 7, hours(2));
        let fresh = left_temp(&recordings, files[0].as_str(), 8, Duration::ZERO);
        let old_state = left_temp(&state_dir, "current-recording", 7, hours(2));

        let s = setup(shared, config(0), &state, false).await;

        assert!(!old.exists() && !old_state.exists());
        assert!(fresh.exists());
        assert_eq!(s.tray().len(), 1);
    })
    .await;
}
