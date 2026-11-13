//! The overlays on every OS: Rune panels fed by `assistant::subscribe`.
//! macOS also keeps them out of Mission Control, orders outlines among
//! app windows, and breathes the frame in Core Animation; Linux and Windows
//! draw the same panels above everything, read the displays from X11 or
//! Win32 ([`displays`]), and leave Escape to the listener.
//!
//! - One frame panel per display, shown while a live task works on the
//!   screen (`Board::frame_until`), and one outline panel per window
//!   a live task works in; the [`watcher`] thread places both.
//! - The orb: Background tasks' rows, questions (answered there by voice or
//!   typed), results, and list, beside the HUD or where the person dragged
//!   it (`place`). It takes the keyboard only while its answer field shows
//!   or after a press opened its list (Escape and a click outside fold it).
//! - The Here question's bubble, just past the dictation HUD, and the
//!   "Finish setting up" bubble in its place ([`setup_needed`]).
//! - The hint past the HUD while the Action shortcut is held, and the
//!   "Hands off" pill while a task moves the real pointer (Esc stops it).
//!
//! Every panel is click-through except the orb's and the bubble's own
//! parts, joins every Space and full-screen Space, is left out of screen
//! captures (so the agents' screenshots stay clean), and holds no window or
//! GPU surface while hidden. The service sends nothing while the assistant
//! is off, and nothing floats before its setup is done.

mod answer;
mod displays;
mod enlarged;
mod follow;
mod orb_panel;
mod wall;
mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSWindow, NSWindowCollectionBehavior};
use rune::{
    DisplayTarget, KeyPolicy, OverlayHandle, OverlayInput, OverlayOptions, OverlaySpec, TextEdit,
    Tx,
};

use super::ask::{Reply, ReplyActions};
use super::board::{Board, Look, Outlined};
use super::bubble::{self, BubbleOverlay};
use super::frame::{self, Frame};
use super::hands::{self, Hands};
use super::hint::{self, Hint};
use super::listening::{self, Listening};
use super::orb::{self, Orb};
use super::outline::{self, Outline};
use super::{jobs, results, stack, when};
use crate::assistant::{ConversationId, Event, TaskWindow};
pub(crate) use answer::setup_needed;
use answer::{
    bubble_event, bubble_poll, dismiss_bubble, place_bubble, want_escape, want_escape_list,
};
use follow::{drag_orb, place_orb, place_past_hud};
pub(crate) use orb_panel::toggle_tasks;
use orb_panel::{
    ask_again, board_changed, fold, hover_task, orb_event, orb_focus, orb_key, orb_poll, rearm,
    reply_edit, retry, send_reply, start_reply, toggle_pressed,
};
#[cfg(test)]
pub(super) use orb_panel::{escape, should_chime};

/// How the orb's parts, outlines, pills, and bubble come and go.
const FADE: Duration = Duration::from_millis(160);

struct Overlays {
    frames: Vec<OverlayHandle<Frame>>,
    outlines: Vec<OverlayHandle<Outline>>,
    orb: OverlayHandle<Orb>,
    bubble: OverlayHandle<BubbleOverlay>,
    hint: OverlayHandle<Hint>,
    hands: OverlayHandle<Hands>,
    listening: OverlayHandle<Listening>,
    /// A task's browser, live and large (`enlarged`).
    enlarged: OverlayHandle<super::enlarged::Enlarged>,
    /// Every browser of a job and its helpers, live (`wall`).
    wall: OverlayHandle<super::wall::Wall>,
}

static OVERLAYS: OnceLock<Overlays> = OnceLock::new();

/// Opens a conversation in the Assistant pane ([`set_open_chats`]).
type OpenChats = Box<dyn Fn(&str) + Send + Sync>;
static OPEN_CHATS: OnceLock<OpenChats> = OnceLock::new();
/// Opens the assistant's setup; set by whoever owns it ([`set_open_setup`]).
type OpenSetup = Box<dyn Fn() + Send + Sync>;
static OPEN_SETUP: OnceLock<OpenSetup> = OnceLock::new();

/// Lets the Assistant pane open a conversation when a row, a bubble, or the
/// Here bubble's "Show in Assistant" is pressed. Called on the main thread
/// with the conversation's id. Until it's set, a press brings the window
/// forward.
pub(crate) fn set_open_chats(open: impl Fn(&str) + Send + Sync + 'static) {
    let _ = OPEN_CHATS.set(Box::new(open));
}

/// What "Continue setup" on the "Finish setting up" bubble does (the same
/// as the trigger's own hook). Called on the main thread. Until it's set,
/// a press brings Hex's window forward.
pub(crate) fn set_open_setup(open: impl Fn() + Send + Sync + 'static) {
    let _ = OPEN_SETUP.set(Box::new(open));
}

fn open_chats(id: &str) {
    match OPEN_CHATS.get() {
        Some(open) => open(id),
        None => crate::rune_ui::window_link::bring_forward(),
    }
}

fn open_setup() {
    match OPEN_SETUP.get() {
        Some(open) => open(),
        None => crate::rune_ui::window_link::bring_forward(),
    }
}

fn options(input: OverlayInput) -> OverlayOptions {
    OverlayOptions {
        input,
        exclude_from_capture: true,
        display: DisplayTarget::Primary,
        ..Default::default()
    }
}

/// A click-through panel's spec.
fn passive<T: 'static>(
    title: &'static str,
    size: (f32, f32),
    view: fn(&T, &mut rune::Cx<T>) -> rune::El,
) -> rune::ConfiguredOverlay<T> {
    OverlaySpec::new(title, size, view)
        .options(options(OverlayInput::None))
        .on_created(|created| leave_out_of_mission_control(created.window_number))
        .release_gpu_when_hidden()
}

/// Adds the overlays to the app and subscribes them to the assistant.
pub(crate) fn install<S: 'static>(app: &mut rune::App<S>) {
    let frames = (0..watcher::MAX_DISPLAYS)
        .map(|_| {
            app.overlay(
                passive("Hex agent frame", (1440.0, 900.0), frame::view),
                Frame::default(),
            )
        })
        .collect();
    let outlines = (0..watcher::MAX_OUTLINES)
        .map(|_| {
            app.overlay(
                passive("Hex agent window", (400.0, 300.0), outline::view),
                Outline::default(),
            )
        })
        .collect();
    let orb_spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Hex assistant tasks", orb::SIZE, orb::view)
    }
    .options(OverlayOptions {
        // Only on request: the answer field, or a list a press opened.
        keys: KeyPolicy::OnDemand,
        ..options(OverlayInput::Regions)
    })
    .poll(orb_poll)
    .on_key(orb_key)
    .on_focus(orb_focus)
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .release_gpu_when_hidden();
    let orb = app.overlay(
        orb_spec,
        Orb::new(
            Board::seeded(crate::assistant::tasks()),
            answer_key(),
            orb::Actions {
                list: stack::Actions {
                    stop: |_, id| crate::assistant::cancel(id),
                    take_over: |_, id| crate::assistant::service::take_over(id),
                    hand_back: |_, id| crate::assistant::service::hand_back(id),
                    stop_all: |_| crate::assistant::service::stop_all(),
                    toggle: toggle_pressed,
                    dismiss: |orb, id| {
                        orb.board.dismiss_question(id);
                        if orb.reply.as_ref().is_some_and(|reply| reply.id == id) {
                            orb.reply = None;
                        }
                    },
                    hover: hover_task,
                    open_preview: |orb, task, session| {
                        enlarged::open(&orb.board, task, session);
                    },
                    defer: |orb, id| {
                        if orb.reply.as_ref().is_some_and(|reply| reply.id == id) {
                            orb.reply = None;
                        }
                        crate::assistant::defer_question(id);
                    },
                    reply: ReplyActions {
                        start: start_reply,
                        cancel: |orb| orb.reply = None,
                        send: send_reply,
                        edit: reply_edit,
                    },
                    results: results::Actions {
                        open: |_, target| results::open(target),
                        retry: |orb, id| {
                            retry(id);
                            orb.open_result = None;
                        },
                        dismiss: |orb, key| {
                            orb.board.pops.dismiss(key);
                            if orb.open_result == Some(key) {
                                orb.open_result = None;
                            }
                            rearm();
                        },
                        expand: |orb, key| {
                            let open = orb.open_result != Some(key);
                            if orb.board.pops.hold(key, open, Instant::now()) {
                                orb.open_result = open.then_some(key);
                            }
                            rearm();
                        },
                        recall: |orb, id| {
                            if let Some(key) = orb.board.recall(id, Instant::now()) {
                                orb.open_result = Some(key);
                                fold(orb);
                            }
                            rearm();
                        },
                    },
                },
                hover: |orb, inside| {
                    if !inside {
                        hover_task(orb, "", false);
                    }
                },
                drag: drag_orb,
                jobs: jobs::LineActions {
                    wall: |orb, id| wall::open(&orb.board, id),
                    all: |orb| wall::open_all(&orb.board),
                },
            },
        ),
    );
    let bubble_spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Hex quick answer", bubble::SIZE, bubble::view)
    }
    .options(options(OverlayInput::Regions))
    .poll(bubble_poll)
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .release_gpu_when_hidden();
    let bubble = app.overlay(
        bubble_spec,
        BubbleOverlay {
            bubble: None,
            below: true,
            shown: false,
            speaking: false,
            setup: None,
            answer_key: answer_key(),
            level: None,
            actions: bubble::Actions {
                dismiss: dismiss_bubble,
                open: |state, id| {
                    open_chats(id);
                    dismiss_bubble(state);
                },
                setup: |state| {
                    open_setup();
                    dismiss_bubble(state);
                },
                settings: |state| {
                    crate::rune_ui::window_link::open_assistant_settings();
                    dismiss_bubble(state);
                },
                retry: |state, id, asked| {
                    ask_again(id, asked);
                    dismiss_bubble(state);
                },
            },
        },
    );
    let hint = app.overlay(
        passive("Hex new task", hint::SIZE, hint::view),
        Hint::default(),
    );
    let hands = app.overlay(
        passive("Hex hands off", hands::SIZE, hands::view),
        Hands::default(),
    );
    let listening = app.overlay(
        passive("Hex listening", listening::SIZE, listening::view).poll(listening_poll),
        Listening::default(),
    );
    // The wall first: a tile pressed opens the enlarged card above it.
    let wall = wall::install(app);
    let enlarged = enlarged::install(app);
    let _ = OVERLAYS.set(Overlays {
        frames,
        outlines,
        orb,
        bubble,
        hint,
        hands,
        listening,
        enlarged,
        wall,
    });
    // Tasks already running when Hex started listening.
    if let Some(overlays) = OVERLAYS.get() {
        overlays
            .orb
            .update(|orb, tx| board_changed(orb, tx, Instant::now()));
    }
    crate::assistant::subscribe(Box::new(on_event));
    // The desk changed (a quick turn left an app or a browser on it, or one
    // was let go): the orb shows it.
    super::minis::watch(|| {
        if let Some(overlays) = OVERLAYS.get() {
            overlays
                .orb
                .update(|orb, tx| board_changed(orb, tx, Instant::now()));
        }
    });
    // A hovered task's browsers coming, going, or showing their first frame.
    super::live::on_change(|| {
        if let Some(overlays) = OVERLAYS.get() {
            overlays.orb.update(|_, _| {});
        }
        enlarged::redraw();
        wall::redraw();
    });
    // The Action shortcut's double tap unfolds the list.
    crate::assistant::shortcut::set_toggle_tasks(toggle_tasks);
    // Displays coming, going, or moving: frames, outlines, and the HUD-side
    // panels follow at once.
    displays::watch(|| {
        watcher::displays_changed();
        follow::follow(true);
    });
    if std::env::var_os("HEX_ASSISTANT_FAKE").is_some() {
        play_fixture();
    }
}

/// `HEX_ASSISTANT_FAKE=1` (development): plays `fixture::script` into the
/// real overlays, with its windows laid out on the main display, then
/// finishes the tasks and ends the Background work.
static FAKE: AtomicBool = AtomicBool::new(false);

fn play_fixture() {
    FAKE.store(true, Ordering::Release);
    let _ = std::thread::Builder::new()
        .name("hex-assistant-fake".into())
        .spawn(|| {
            let start = Instant::now();
            for (at, event) in super::fixture::script_to_the_end() {
                let due = start + Duration::from_secs_f64(at);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                on_event(&event);
            }
        });
}

/// Where a task's window is: Accessibility's answer, or the fixture's
/// layout while it plays.
fn window_frame(
    window: &TaskWindow,
    displays: &[super::screens::Screen],
) -> Option<(f64, f64, f64, f64)> {
    if FAKE.load(Ordering::Acquire) {
        let main = displays.first()?.frame;
        let (x, y, w, h) = super::fixture::frame(window, (main[2], main[3]))?;
        return Some((main[0] + x, main[1] + y, w, h));
    }
    crate::assistant::computer::window_frame(window)
}

/// The subscriber: on the service's thread (or the watcher's), so it only
/// queues work for the overlays' (main) thread and the watcher.
fn on_event(event: &Event) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    match event {
        // The frame follows screen work on the board (`Board::frame_until`),
        // not background work in general.
        Event::ConversationsChanged => {}
        _ => {
            let for_orb = event.clone();
            overlays
                .orb
                .update_if(move |orb, tx| orb_event(orb, &for_orb, tx));
            let for_bubble = event.clone();
            overlays
                .bubble
                .update_if(move |state, tx| bubble_event(state, &for_bubble, tx));
        }
    }
}

// ---------------------------------------------------------------------------
// The hint past the HUD

/// What the hint shows now: `None`, or whether it's the Background pill.
static HINT_SHOWN: Mutex<Option<bool>> = Mutex::new(None);

/// Whether the hint shows now.
fn hint_shown() -> bool {
    HINT_SHOWN
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_some()
}

/// What the HUD says past itself while the Action shortcut is held:
/// `asking` is `Some(true)` for a Background task (tap+hold: "New
/// background task"), `Some(false)` for a Here question ("Here · Mail"),
/// `None` once the hold ends. The HUD calls it on the main thread with each
/// event, once it has placed itself; a repeat does nothing. Nothing shows
/// before the assistant's setup is done.
pub(crate) fn ask_hint(asking: Option<bool>) {
    let mut shown = HINT_SHOWN.lock().unwrap_or_else(|error| error.into_inner());
    if *shown == asking {
        return;
    }
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let hint = match asking {
        Some(true) if ready() => Some(Hint::Background),
        Some(false) if ready() => frontmost_app().map(Hint::Here),
        _ => None,
    };
    *shown = asking;
    match hint {
        Some(hint) => {
            overlays.hint.update(move |state, _| *state = hint);
            place_past_hud(&overlays.hint, hint::SIZE);
            overlays.hint.fade_in(FADE);
        }
        None => overlays.hint.fade_out(FADE),
    }
}

/// The Action shortcut's keys as a person reads them ("⌘"), for "Hold ⌘
/// to answer".
fn answer_key() -> String {
    crate::app_settings::assistant_runtime()
        .and_then(|(_, settings)| settings.action_hotkey)
        .map(|binding| binding.keycaps().join(""))
        .filter(|keys| !keys.is_empty())
        .unwrap_or_else(|| "⌘".into())
}

/// Whether the assistant is on and set up.
fn ready() -> bool {
    crate::app_settings::assistant_runtime()
        .is_some_and(|(on, settings)| on && (settings.setup_done || crate::assistant::trying()))
}

/// The app in front, which a Here question is about.
#[cfg(target_os = "macos")]
fn frontmost_app() -> Option<String> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(app.localizedName()?.to_string()).filter(|name| !name.is_empty())
}

/// The app in front, which a Here question is about.
#[cfg(not(target_os = "macos"))]
fn frontmost_app() -> Option<String> {
    crate::assistant::computer::quick_context().frontmost_app
}

// ---------------------------------------------------------------------------
// Listening (conversation mode)

/// How often the listening ring moves on.
const LISTENING_TICK: Duration = Duration::from_millis(80);

/// Shows conversation mode's listening past the HUD on the display the
/// person works on while `on` (the trigger's `Listening`, until speech
/// starts or it gives up). The HUD calls it on the main thread; a repeat
/// does nothing. Nothing shows before the assistant's setup is done.
pub(crate) fn listening(on: bool) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let seconds = crate::app_settings::assistant_runtime()
        .map_or(8, |(_, settings)| settings.conversation_seconds) as f32;
    // One source of truth (`interaction::listening`): the ring shows exactly
    // while conversation mode's capture is open (waiting or hearing); a
    // hold shows in the HUD itself. The HUD's own event is only the cue.
    let _ = on;
    let on = matches!(
        crate::assistant::interaction::listening(),
        crate::assistant::interaction::Listening::FollowUp
            | crate::assistant::interaction::Listening::Hearing
    ) && ready()
        && seconds > 0.0;
    overlays.listening.update_if(move |state, tx| {
        let Some(overlays) = OVERLAYS.get() else {
            return false;
        };
        match (on, state.since.is_some()) {
            (true, false) => {
                *state = Listening::start(Instant::now(), seconds);
                place_past_hud(&overlays.listening, listening::SIZE);
                overlays.listening.fade_in(FADE);
                tx.wake_after(LISTENING_TICK);
                true
            }
            (false, true) => {
                state.since = None;
                overlays.listening.fade_out(FADE);
                false
            }
            _ => false,
        }
    });
}

fn listening_poll(state: &mut Listening, _clock: f64, tx: &mut Tx) -> bool {
    if state.since.is_none() {
        return false;
    }
    let changed = state.tick(Instant::now());
    if state.left > 0.0 {
        tx.wake_after(LISTENING_TICK);
    }
    changed
}

// ---------------------------------------------------------------------------
// Hands off

/// The task moving the real pointer, while the pill shows.
static HANDS: Mutex<Option<ConversationId>> = Mutex::new(None);

fn hands_task() -> Option<ConversationId> {
    HANDS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// Shows the pill while `task` moves the real pointer, with Esc stopping
/// it, and makes room for it above the Here bubble. Main thread.
fn set_hands(task: Option<ConversationId>) {
    {
        let mut hands = HANDS.lock().unwrap_or_else(|error| error.into_inner());
        if *hands == task {
            return;
        }
        let same_pill = hands.is_some() == task.is_some();
        *hands = task.clone();
        if same_pill {
            return;
        }
    }
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let on = task.is_some();
    if on {
        let shown = task.clone();
        overlays.hands.update(move |state, _| state.task = shown);
        place_past_hud(&overlays.hands, hands::SIZE);
        overlays.hands.fade_in(FADE);
    } else {
        overlays.hands.fade_out(FADE);
    }
    want_escape(None, Some(on));
    overlays.bubble.update_if(|state, _| {
        if state.shown {
            state.below = place_bubble(&OVERLAYS.get().expect("installed").bubble);
        }
        false
    });
}

// ---------------------------------------------------------------------------
// Native touches on the panels (main thread)

#[cfg(target_os = "macos")]
fn panel(number: Option<u32>) -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    let number = number? as isize;
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .find(|window| window.windowNumber() == number)
}

/// Linux and Windows have no Mission Control to leave.
#[cfg(not(target_os = "macos"))]
fn leave_out_of_mission_control(_number: Option<u32>) {}

/// Hides the panel from Mission Control and Exposé (Transient) while it
/// still joins every Space and full-screen Space; Rune's own mask keeps it
/// in place there (Stationary) instead.
#[cfg(target_os = "macos")]
fn leave_out_of_mission_control(number: Option<u32>) {
    let Some(panel) = panel(number) else {
        return;
    };
    let behavior = (panel.collectionBehavior() - NSWindowCollectionBehavior::Stationary)
        | NSWindowCollectionBehavior::Transient
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::IgnoresCycle;
    panel.setCollectionBehavior(behavior);
}
