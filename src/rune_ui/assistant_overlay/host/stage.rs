//! The stage's panel: made with the other overlays, shown under the island
//! on the display under the pointer, keyed while a press opened it
//! (Escape, ↑ ↓, Space), and folded away [`stage::STAYS`] after nobody
//! looks at it (never while pinned, pointed at, or read aloud). What
//! projects there: the Here bubble's More, a notification's click, a
//! job's result from the orb (More, or a press in its Done group), the
//! enlarged card's What it did, and the repair check's card. Voice acts on
//! what it shows ([`takes`]).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use rune::{
    Align, DisplayTarget, KeyPolicy, OverlayHandle, OverlayInput, OverlayPlacement, OverlaySpec,
    ScreenEdge, Tx,
};

use super::super::board::{Board, Phase};
use super::super::results::{self, Outcome, Target};
use super::super::stage::{self, Actions, Answer, InPlace, Panel, RepairAsk, Stage, Steps, Verb};
use super::{FADE, OVERLAYS, answer_key, leave_out_of_mission_control, options};
use crate::desktop::setup_gaps::Gap;

/// How often a shown panel looks whether it's time to fold.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// What the stage shows to open, for voice, off the main thread.
static TARGETS: Mutex<Vec<Target>> = Mutex::new(Vec::new());
/// When the panel was last held (shown, pointed at, read aloud).
static HELD_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Adds the panel to the app.
pub(super) fn install<S: 'static>(app: &mut rune::App<S>) -> OverlayHandle<Stage> {
    let spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Agent-Desk stage", stage::SIZE, stage::view)
    }
    .options(rune::OverlayOptions {
        keys: KeyPolicy::OnDemand,
        display: DisplayTarget::UnderPointer,
        placement: Some(OverlayPlacement::Edge {
            edge: ScreenEdge::Top,
            margin: stage::TOP,
            align: Align::Center,
            inset: 0.0,
        }),
        ..options(OverlayInput::Regions)
    })
    .poll(poll)
    .on_key(key)
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .release_gpu_when_hidden();
    app.overlay(
        spec,
        Stage::new(
            answer_key(),
            Actions {
                close,
                open: |_, target, verb| open(target, verb),
                save,
                retry: |state, id| {
                    super::retry(id);
                    close(state);
                },
                window: |state, id| {
                    // The person asked for the window, here and only here.
                    super::open_chats(id);
                    close(state);
                },
                repair: |state, ask| {
                    repair(ask);
                    close(state);
                },
            },
        ),
    )
}

/// Esc puts it away; ↑ ↓ and Space scroll what it shows.
fn key(state: &mut Stage, key: &rune::KeyEvent, tx: &mut Tx) -> bool {
    let Some(panel) = state.panel.as_ref() else {
        return false;
    };
    let scroll = match panel {
        Panel::Answer(_) => "stage-answer",
        Panel::Result { .. } => "stage-preview",
        Panel::Steps(_) => "stage-steps-body",
        Panel::Repair(_) => "",
    };
    match &key.key {
        rune::Key::Escape => {
            if state.menu {
                state.menu = false;
            } else {
                close(state);
            }
            true
        }
        rune::Key::Down | rune::Key::PageDown if !scroll.is_empty() => {
            tx.scroll_page(scroll, 1);
            true
        }
        rune::Key::Text(space) if space == " " && !scroll.is_empty() => {
            tx.scroll_page(scroll, if key.mods.shift { -1 } else { 1 });
            true
        }
        rune::Key::Up | rune::Key::PageUp if !scroll.is_empty() => {
            tx.scroll_page(scroll, -1);
            true
        }
        _ => false,
    }
}

fn poll(state: &mut Stage, _clock: f64, tx: &mut Tx) -> bool {
    if state.panel.is_none() {
        return false;
    }
    let now = Instant::now();
    let held = state.hovered || crate::assistant::speech::speaking();
    let mut at = HELD_AT.lock().unwrap_or_else(|error| error.into_inner());
    if held || at.is_none() {
        *at = Some(now);
    }
    let since = at.unwrap_or(now);
    drop(at);
    match stage::folds_at(since, state.pinned, held) {
        Some(due) if now >= due => {
            close(state);
            true
        }
        _ => {
            tx.wake_after(LOOK_EVERY);
            false
        }
    }
}

/// Puts `panel` on the stage, in place of what showed, and takes the
/// keyboard when a press brought it (never for the repair card).
pub(crate) fn show(panel: Panel) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let keys = panel.takes_keys();
    *TARGETS.lock().unwrap_or_else(|error| error.into_inner()) = panel.targets();
    *HELD_AT.lock().unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
    let key = answer_key();
    overlays.stage.update(move |state, tx| {
        state.answer_key = key;
        state.show(panel);
        tx.wake_after(LOOK_EVERY);
    });
    overlays.stage.fade_in(FADE);
    if keys {
        overlays.stage.request_keys();
    }
}

/// ×, Esc, or folding away: the panel goes.
fn close(state: &mut Stage) {
    state.panel = None;
    state.pinned = false;
    state.menu = false;
    TARGETS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    if let Some(overlays) = OVERLAYS.get() {
        overlays.stage.fade_out(FADE);
    }
}

/// The Here bubble's More: the whole answer on the stage.
pub(super) fn show_answer(id: &str, question: &str, text: &str) {
    show(Panel::Answer(Answer {
        id: id.to_owned(),
        question: question.to_owned(),
        text: text.to_owned(),
    }));
}

/// Job `id`'s result (a notification's click, More on its bubble, a press
/// in the orb's Done group), from the orb's board, else from its saved
/// conversation.
pub(crate) fn show_result(id: &str) {
    #[cfg(test)]
    PROJECTED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(id.to_owned());
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let id = id.to_owned();
    overlays.orb.update_if(move |orb, _| {
        let panel = result_of(&orb.board, &id).or_else(|| saved_result(&id));
        match panel {
            Some(panel) => show(panel),
            None => tracing::debug!(id, "nothing to project for this conversation"),
        }
        false
    });
}

/// Every conversation [`show_result`] was asked to project (tests).
#[cfg(test)]
pub(crate) static PROJECTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What `board` knows of job `id`'s end: its result and its steps.
pub(super) fn result_of(board: &Board, id: &str) -> Option<Panel> {
    let task = board.view.task(id);
    let outcome = board.outcome(id).cloned().or_else(|| {
        let task = task?;
        match task.phase {
            Phase::Done => Some(Outcome::of(task, false, Instant::now())),
            Phase::Failed => Some(Outcome::of(task, true, Instant::now())),
            _ => None,
        }
    })?;
    let steps = task.map(steps_of).unwrap_or_default();
    let log = log_of(&outcome);
    Some(Panel::Result {
        outcome,
        steps,
        log,
    })
}

/// A task's steps in full, as it reported them (its windows named too).
fn steps_of(task: &crate::assistant::view::Task) -> Vec<String> {
    use crate::assistant::view::Logged;
    let full: Vec<String> = task
        .log
        .iter()
        .map(|logged| match logged {
            Logged::Step(step) => step.clone(),
            Logged::Window(app) => crate::desktop::i18n::tr_fill("Worked in {}", app),
        })
        .collect();
    if full.is_empty() {
        task.steps.clone()
    } else {
        full
    }
}

/// The job's own record of what it did, when its folder keeps one.
fn log_of(outcome: &Outcome) -> Option<String> {
    outcome
        .targets()
        .iter()
        .filter(|target| target.kind == results::Kind::Folder)
        .map(|target| std::path::Path::new(&target.open).join(crate::assistant::audit::IN_FOLDER))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// A conversation the board no longer has (Agent-Desk started since): its
/// last answer, as saved.
fn saved_result(id: &str) -> Option<Panel> {
    let stored = crate::assistant::store::Store::open().ok()?.load(id)?;
    let text = stored
        .conversation
        .messages
        .iter()
        .rev()
        .find(|message| message.role == crate::assistant::Role::Assistant)
        .map(crate::assistant::Message::text)?;
    Some(Panel::Result {
        outcome: Outcome {
            id: id.to_owned(),
            title: stored.conversation.title.clone(),
            colour: 0,
            failed: false,
            text,
            at: Instant::now(),
        },
        steps: Vec::new(),
        log: None,
    })
}

/// The enlarged card's What it did: task `id`'s steps.
pub(super) fn show_steps(board: &Board, id: &str) {
    let Some(task) = board.view.task(id) else {
        return;
    };
    show(Panel::Steps(Steps {
        id: id.to_owned(),
        title: task.title.clone(),
        colour: task.colour,
        steps: steps_of(task),
        log: None,
        back: None,
    }));
}

/// The repair check found something missing again: a small card, not the
/// window. Any thread.
pub(crate) fn needs_repair(gaps: Vec<Gap>) {
    if gaps.is_empty() {
        return;
    }
    show(Panel::Repair(gaps));
}

/// What a short spoken request asks of the stage's targets, done here:
/// true when it took the words ("open the folder"), so they don't go to
/// the assistant. Any thread.
pub(crate) fn takes(words: &str) -> bool {
    let targets = TARGETS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let Some((at, verb)) = stage::spoken(words, &targets) else {
        return false;
    };
    let target = targets[at].open.clone();
    open(&target, verb);
    true
}

/// A target's verb: open it, or show it in its folder.
fn open(target: &str, verb: Verb) {
    match verb {
        Verb::Open | Verb::OpenFolder | Verb::OpenLink => results::open(target),
        Verb::Show => reveal(target),
    }
}

/// Shows `path` selected in Finder (its folder elsewhere).
fn reveal(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let path = path.to_owned();
        let _ = std::thread::Builder::new()
            .name("hex-assistant-reveal".into())
            .spawn(move || {
                if let Err(error) = crate::desktop::background_process::command("/usr/bin/open")
                    .arg("-R")
                    .arg(&path)
                    .status()
                {
                    tracing::warn!(%error, "assistant: couldn't show a result in Finder");
                }
            });
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(folder) = std::path::Path::new(path).parent() {
        results::open(&folder.to_string_lossy());
    }
}

/// Save as note: the answer as Markdown in `~/Documents/Agent-Desk Notes`,
/// then shown in Finder.
fn save(state: &mut Stage) {
    let Some((title, text)) = state.panel.as_ref().and_then(stage::note_text) else {
        return;
    };
    let Some(folder) = dirs::document_dir().map(|dir| dir.join("Agent-Desk Notes")) else {
        return;
    };
    let name = stage::note_name(&title);
    let written = std::fs::create_dir_all(&folder).and_then(|()| {
        let mut path = folder.join(&name);
        let stem = name.trim_end_matches(".md").to_owned();
        let mut n = 2;
        while path.exists() {
            path = folder.join(format!("{stem} {n}.md"));
            n += 1;
        }
        std::fs::write(&path, text).map(|()| path)
    });
    match written {
        Ok(path) => {
            state.saved = true;
            reveal(&path.to_string_lossy());
        }
        Err(error) => tracing::warn!(%error, "assistant: couldn't save the answer as a note"),
    }
}

/// The repair card's buttons.
fn repair(ask: RepairAsk) {
    match ask {
        RepairAsk::Fix(fix) => fix_in_place(fix),
        RepairAsk::FixAll => super::repair_hook(super::Repair::FixAll),
        RepairAsk::Later => super::repair_hook(super::Repair::Later),
    }
}

/// Fixes what can be fixed without the window: one System Settings pane,
/// or the agent's sign-in in Terminal.
pub(super) fn fix_in_place(fix: InPlace) {
    let _ = std::thread::Builder::new()
        .name("hex-assistant-fix".into())
        .spawn(move || match fix {
            InPlace::SignIn => {
                let backend = crate::app_settings::assistant_runtime()
                    .map(|(_, settings)| settings.backend)
                    .unwrap_or_default();
                if let Err(error) = crate::assistant::backends::apply_fix(
                    crate::assistant::backends::Fix::SignIn(backend),
                ) {
                    tracing::warn!(%error, "assistant: the sign-in didn't finish");
                }
            }
            pane => {
                #[cfg(target_os = "macos")]
                {
                    let anchor = match pane {
                        InPlace::Accessibility => "Privacy_Accessibility",
                        InPlace::ScreenRecording => "Privacy_ScreenCapture",
                        InPlace::Microphone => "Privacy_Microphone",
                        InPlace::Automation => "Privacy_Automation",
                        InPlace::SignIn => unreachable!(),
                    };
                    let url =
                        format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
                    if let Err(error) = crate::desktop::background_process::command("/usr/bin/open")
                        .arg(url)
                        .status()
                    {
                        tracing::warn!(%error, "assistant: couldn't open System Settings");
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = pane;
            }
        });
}
