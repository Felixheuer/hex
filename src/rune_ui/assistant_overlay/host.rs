//! The overlays on macOS: Rune panels fed by `assistant::subscribe`.
//!
//! - One frame panel per display, shown while Background work runs, waits,
//!   or is queued (`Event::ModeChanged`), and one outline panel per window
//!   a live task works in; the [`watcher`] thread places both.
//! - The orb: Background tasks' bubbles and list, beside the HUD or where
//!   the person dragged it (`place`).
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

mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSWindow, NSWindowCollectionBehavior};
use rune::{
    Align, DisplayTarget, OverlayHandle, OverlayInput, OverlayOptions, OverlayPlacement,
    OverlaySpec, ScreenEdge, Tx,
};

use super::board::{Board, Bubble, Due, Look, Outlined};
use super::bubble::{self, BubbleOverlay};
use super::frame::{self, Frame};
use super::hands::{self, Hands};
use super::hint::{self, Hint};
use super::orb::{self, Orb};
use super::outline::{self, Outline};
use super::place::{self, Area};
use super::{stack, when};
use crate::assistant::{ConversationId, Event, TaskWindow};
use crate::rune_ui::live_text::Spot;

/// How the orb's parts, outlines, pills, and bubble come and go.
const FADE: Duration = Duration::from_millis(160);
/// The bubble's panel fades a little slower, so its card can shrink back.
const BUBBLE_OUT: Duration = Duration::from_millis(260);
/// How long "Finish setting up the assistant first" stays.
const SETUP_SHOWS: Duration = Duration::from_secs(7);
/// Esc's key code.
const ESCAPE: u16 = 53;

struct Overlays {
    frames: Vec<OverlayHandle<Frame>>,
    outlines: Vec<OverlayHandle<Outline>>,
    orb: OverlayHandle<Orb>,
    bubble: OverlayHandle<BubbleOverlay>,
    hint: OverlayHandle<Hint>,
    hands: OverlayHandle<Hands>,
}

static OVERLAYS: OnceLock<Overlays> = OnceLock::new();

/// Opens a conversation in Chats; set by the Chats pane ([`set_open_chats`]).
type OpenChats = Box<dyn Fn(&str) + Send + Sync>;
static OPEN_CHATS: OnceLock<OpenChats> = OnceLock::new();
/// Opens the assistant's setup; set by whoever owns it ([`set_open_setup`]).
type OpenSetup = Box<dyn Fn() + Send + Sync>;
static OPEN_SETUP: OnceLock<OpenSetup> = OnceLock::new();

/// Lets the Chats pane open a conversation when a row, a bubble, or the
/// Here bubble's "Open in Chats" is pressed. Called on the main thread with
/// the conversation's id. Until it's set, a press brings the window forward.
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
        None => {
            tracing::info!(
                conversation = id,
                "assistant: Chats isn't wired yet; showing the window"
            );
            crate::macos_rune::bring_window_forward();
        }
    }
}

fn open_setup() {
    match OPEN_SETUP.get() {
        Some(open) => open(),
        None => crate::macos_rune::bring_window_forward(),
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
    .options(options(OverlayInput::Regions))
    .poll(orb_poll)
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .release_gpu_when_hidden();
    let orb = app.overlay(
        orb_spec,
        Orb {
            board: Board::seeded(crate::assistant::tasks()),
            shown: false,
            armed: None,
            expanded: false,
            peek: false,
            grow: orb::Grow::default(),
            actions: orb::Actions {
                list: stack::Actions {
                    stop: |_, id| crate::assistant::cancel(id),
                    open: |_, id| open_chats(id),
                    take_over: |_, id| crate::assistant::service::take_over(id),
                    hand_back: |_, id| crate::assistant::service::hand_back(id),
                    stop_all: |_| crate::assistant::service::stop_all(),
                    toggle: toggle,
                },
                hover: |orb, inside| orb.peek = inside && !orb.board.stack().is_empty(),
                drag: drag_orb,
            },
        },
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
    let _ = OVERLAYS.set(Overlays {
        frames,
        outlines,
        orb,
        bubble,
        hint,
        hands,
    });
    // Tasks already running when Hex started listening.
    if let Some(overlays) = OVERLAYS.get() {
        overlays
            .orb
            .update(|orb, tx| board_changed(orb, tx, Instant::now()));
    }
    if crate::assistant::service::background_busy() {
        watcher::set_frame(true);
    }
    crate::assistant::subscribe(Box::new(on_event));
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
    displays: &[watcher::Display],
) -> Option<(f64, f64, f64, f64)> {
    if FAKE.load(Ordering::Acquire) {
        let main = displays.first()?.bounds;
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
        Event::ModeChanged { on } => watcher::set_frame(*on),
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
// The orb (and the board the outlines and the Hands off pill read)

/// Unfolds the task list from the orb, or folds it back (a double-tap of
/// the Action shortcut). Any thread; nothing while no Background task is
/// listed.
#[allow(dead_code)] // The Action shortcut's double-tap calls it.
pub(crate) fn toggle_tasks() {
    if let Some(overlays) = OVERLAYS.get() {
        overlays.orb.update(|orb, _| toggle(orb));
    }
}

fn toggle(orb: &mut Orb) {
    if orb.board.stack().is_empty() {
        orb.expanded = false;
        return;
    }
    orb.expanded = !(orb.expanded || orb.peek);
    orb.peek = false;
}

fn orb_event(orb: &mut Orb, event: &Event, tx: &mut Tx) -> bool {
    let colour = match event {
        Event::Queued { conversation, .. } | Event::Started { conversation, .. } => {
            colour_of(conversation)
        }
        _ => None,
    };
    let now = Instant::now();
    if !orb.board.apply(event, colour, now) {
        return false;
    }
    board_changed(orb, tx, now);
    true
}

/// The service's colour for a task, read here on the main thread rather
/// than in the subscriber, which runs under the service.
fn colour_of(conversation: &ConversationId) -> Option<u8> {
    crate::assistant::tasks()
        .into_iter()
        .find(|task| &task.conversation == conversation)
        .map(|task| task.colour)
}

fn board_changed(orb: &mut Orb, tx: &mut Tx, now: Instant) {
    let mut outlined = orb.board.outlined();
    let hands = orb.board.hands_off().cloned();
    if let Some(task) = &hands
        && task.windows.is_empty()
    {
        // A Here task names no window of its own: the one it holds.
        outlined.extend(
            crate::assistant::guard::leases()
                .into_iter()
                .filter(|lease| lease.task == task.id)
                .map(|lease| Outlined {
                    task: task.id.clone(),
                    colour: task.colour,
                    title: task.title.clone(),
                    window: lease.window,
                    look: Look::HandsOff,
                }),
        );
    }
    let live = orb.board.tasks.iter().any(|task| task.phase.live());
    watcher::publish(outlined, live);
    set_hands(hands.map(|task| task.id));
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    if orb.board.stack().is_empty() {
        orb.expanded = false;
        orb.peek = false;
    }
    let visible = orb.board.orb_shows();
    if visible && !orb.shown {
        place_orb(&overlays.orb, orb);
        overlays.orb.fade_in(FADE);
        orb.shown = true;
    } else if !visible && orb.shown {
        overlays.orb.fade_out(FADE);
        orb.shown = false;
    }
    arm(orb, tx, now);
}

/// Wakes the orb when its next finished task or bubble is due to leave, or
/// a waiting task's countdown next changes.
fn arm(orb: &mut Orb, tx: &mut Tx, now: Instant) {
    let now_ms = when::now_ms();
    let tick = orb
        .board
        .tasks
        .iter()
        .filter_map(|task| match task.phase {
            super::board::Phase::Waiting { until_ms } => when::next_tick(until_ms, now_ms),
            _ => None,
        })
        .min()
        .map(|after| now + after);
    let next = orb.board.next_expiry().into_iter().chain(tick).min();
    if let Some(at) = next
        && next != orb.armed
    {
        tx.wake_after(at.saturating_duration_since(now));
    }
    orb.armed = next;
}

fn orb_poll(orb: &mut Orb, _clock: f64, tx: &mut Tx) -> bool {
    let now = Instant::now();
    if orb.board.expire(now) {
        orb.armed = None;
        board_changed(orb, tx, now);
        return true;
    }
    if orb.armed.is_some_and(|at| now >= at) {
        // A countdown's minute, or woken a little early: look again.
        orb.armed = None;
        arm(orb, tx, now);
        return orb.board.waiting();
    }
    false
}

/// Where the orb's centre is now, in global points.
static ORB_AT: Mutex<Option<(f64, f64)>> = Mutex::new(None);

fn set_orb_at(anchor: (f64, f64)) {
    *ORB_AT.lock().unwrap_or_else(|error| error.into_inner()) = Some(anchor);
}

/// The pointer's display: its id, its visible area in global points (y
/// down), and the pointer there.
fn pointer_display() -> Option<(Option<u32>, Area, (f64, f64))> {
    let (screen, primary_top, _, id) =
        MainThreadMarker::new().and_then(crate::rune_hud::pointer_screen)?;
    let v = screen.visible;
    let visible = [v.x, primary_top - (v.y + v.height), v.width, v.height];
    let pointer = NSEvent::mouseLocation();
    Some((id, visible, (pointer.x, primary_top - pointer.y)))
}

/// Puts the orb on the pointer's display: where it was dragged there, else
/// beside the HUD.
fn place_orb(handle: &OverlayHandle<Orb>, orb: &mut Orb) {
    let Some((display, visible, _)) = pointer_display() else {
        return;
    };
    let hud = match crate::rune_hud::live_text_place() {
        Spot::Global { x, below, .. }
            if f64::from(x) >= visible[0] && f64::from(x) < visible[0] + visible[2] =>
        {
            Some((f64::from(x), below))
        }

        Spot::Edge(edge, _) => Some((visible[0] + visible[2] / 2.0, edge == ScreenEdge::Top)),
        Spot::Global { .. } => None,
    };
    let anchor = place::anchor(display, visible, hud);
    orb.grow = place::grow(anchor, visible);
    set_orb_at(anchor);
    set_rect(handle, place::rect(anchor, orb.grow));
}

fn set_rect<T>(handle: &OverlayHandle<T>, [x, y, width, height]: [f32; 4]) {
    handle.set_display(DisplayTarget::Primary);
    handle.set_placement(Some(OverlayPlacement::Rect {
        x,
        y,
        width,
        height,
    }));
}

thread_local! {
    /// While the orb is dragged: the pointer and the orb's centre when it
    /// was pressed.
    static DRAG: std::cell::Cell<Option<((f64, f64), (f64, f64))>> =
        const { std::cell::Cell::new(None) };
}

/// Drags the orb with the pointer; on release, keeps it inside the display
/// it was dropped on, turns what unfolds from it toward that display, and
/// remembers the spot for that display.
fn drag_orb(orb: &mut Orb, drag: &rune::Drag) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let Some((_, _, pointer)) = pointer_display() else {
        return;
    };
    let at = *ORB_AT.lock().unwrap_or_else(|error| error.into_inner());
    match drag.phase {
        rune::Phase::Down => {
            DRAG.set(at.map(|anchor| (pointer, anchor)));
        }
        rune::Phase::Move => {
            if let Some((from, anchor)) = DRAG.get() {
                let moved = (anchor.0 + pointer.0 - from.0, anchor.1 + pointer.1 - from.1);
                set_rect(&overlays.orb, place::rect(moved, orb.grow));
            }
        }
        rune::Phase::Up => {
            let Some((from, anchor)) = DRAG.take() else {
                return;
            };
            let Some((display, visible, _)) = pointer_display() else {
                return;
            };
            let dropped = if drag.cancelled() {
                anchor
            } else {
                place::clamp(
                    (anchor.0 + pointer.0 - from.0, anchor.1 + pointer.1 - from.1),
                    visible,
                )
            };
            orb.grow = place::grow(dropped, visible);
            set_rect(&overlays.orb, place::rect(dropped, orb.grow));
            if let Some(id) = display
                && !drag.cancelled()
            {
                place::remember(id, visible, dropped);
            }
            set_orb_at(dropped);
        }
    }
}

// ---------------------------------------------------------------------------
// The hint past the HUD

/// What the hint shows now: `None`, or whether it's the Background pill.
static HINT_SHOWN: Mutex<Option<bool>> = Mutex::new(None);

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

/// Whether the assistant is on and set up.
fn ready() -> bool {
    crate::app_settings::assistant_runtime().is_some_and(|(on, settings)| on && settings.setup_done)
}

/// The app in front, which a Here question is about.
fn frontmost_app() -> Option<String> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(app.localizedName()?.to_string()).filter(|name| !name.is_empty())
}

/// Places a pill where Live text would start, past its plate when Live text
/// may show one there.
fn place_past_hud<T>(handle: &OverlayHandle<T>, (width, height): (f32, f32)) {
    let past = if crate::labs::live_text() {
        crate::rune_ui::live_text::SIZE.1
    } else {
        0.0
    };
    match crate::rune_hud::live_text_place() {
        Spot::Global { x, y, below } => {
            handle.set_display(DisplayTarget::Primary);
            handle.set_placement(Some(OverlayPlacement::Rect {
                x: x - width / 2.0,
                y: if below { y + past } else { y - height - past },
                width,
                height,
            }));
        }
        Spot::Edge(edge, margin) => {
            handle.set_display(DisplayTarget::UnderPointer);
            handle.set_placement(Some(OverlayPlacement::Edge {
                edge,
                margin: margin + past,
                align: Align::Center,
                inset: 0.0,
            }));
        }
    }
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
// The bubble

fn bubble_event(state: &mut BubbleOverlay, event: &Event, tx: &mut Tx) -> bool {
    let now = Instant::now();
    let before = state.bubble.clone();
    if !Bubble::apply(&mut state.bubble, event, now) {
        return false;
    }
    let Some(overlays) = OVERLAYS.get() else {
        return true;
    };
    match &state.bubble {
        None => {
            // Cancelled: it shrinks away with what it showed.
            state.bubble = before;
            hide_bubble(state);
        }
        Some(bubble) => {
            let fresh = before
                .as_ref()
                .is_none_or(|old| old.conversation != bubble.conversation);
            state.setup = None;
            if fresh || !state.shown {
                show_bubble(state, &overlays.bubble);
            }
            settle_bubble(state, tx, now);
        }
    }
    true
}

fn show_bubble(state: &mut BubbleOverlay, handle: &OverlayHandle<BubbleOverlay>) {
    state.below = place_bubble(handle);
    handle.fade_in(FADE);
    state.shown = true;
    want_escape(Some(true), None);
}

/// "Finish setting up the assistant first", with Continue setup, where the
/// Here bubble shows: the trigger calls this when the Action shortcut is
/// used before the assistant is set up. Any thread; it leaves by itself.
#[allow(dead_code)] // The Action shortcut calls it.
pub(crate) fn setup_needed() {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    overlays.bubble.update(|state, tx| {
        state.bubble = None;
        state.setup = Some(Instant::now() + SETUP_SHOWS);
        if let Some(overlays) = OVERLAYS.get() {
            show_bubble(state, &overlays.bubble);
        }
        tx.wake_after(SETUP_SHOWS);
    });
}

fn bubble_poll(state: &mut BubbleOverlay, _clock: f64, tx: &mut Tx) -> bool {
    settle_bubble(state, tx, Instant::now())
}

/// Hides an answered bubble once its time is up, and follows its speech;
/// whether it changed.
fn settle_bubble(state: &mut BubbleOverlay, tx: &mut Tx, now: Instant) -> bool {
    if !state.shown {
        return false;
    }
    if let Some(until) = state.setup {
        if now >= until {
            hide_bubble(state);
            return true;
        }
        tx.wake_after(until - now);
        return false;
    }
    let speaking = crate::assistant::speech::speaking();
    let voice_changed = state.speaking != speaking;
    state.speaking = speaking;
    let Some(bubble) = state.bubble.as_mut() else {
        return voice_changed;
    };
    match bubble.due(now, speaking) {
        Due::Stay => voice_changed,
        Due::At(at) => {
            tx.wake_after(at.saturating_duration_since(now));
            voice_changed
        }
        Due::Leave => {
            hide_bubble(state);
            true
        }
    }
}

fn hide_bubble(state: &mut BubbleOverlay) {
    if !state.shown {
        return;
    }
    state.shown = false;
    state.speaking = false;
    want_escape(Some(false), None);
    if let Some(overlays) = OVERLAYS.get() {
        overlays.bubble.fade_out(BUBBLE_OUT);
    }
}

/// A press on the card, or Esc: the bubble goes and stops reading aloud.
/// The question itself goes on; its answer is in Chats.
fn dismiss_bubble(state: &mut BubbleOverlay) {
    if state.shown && state.setup.is_none() {
        crate::assistant::speech::stop();
    }
    hide_bubble(state);
}

/// Places the bubble where Live text would show, just past the HUD's plate
/// (and past the Hands off pill while it shows); whether it hangs below
/// that spot.
fn place_bubble(handle: &OverlayHandle<BubbleOverlay>) -> bool {
    let (width, height) = bubble::SIZE;
    let room = if hands_task().is_some() {
        hands::SIZE.1
    } else {
        0.0
    };
    match crate::rune_hud::live_text_place() {
        Spot::Global { x, y, below } => {
            handle.set_display(DisplayTarget::Primary);
            handle.set_placement(Some(OverlayPlacement::Rect {
                x: x - width / 2.0,
                y: if below {
                    y - bubble::INSET + room
                } else {
                    y - height + bubble::INSET - room
                },
                width,
                height,
            }));
            below
        }
        Spot::Edge(edge, margin) => {
            handle.set_display(DisplayTarget::UnderPointer);
            handle.set_placement(Some(OverlayPlacement::Edge {
                edge,
                margin: margin - bubble::INSET + room,
                align: Align::Center,
                inset: 0.0,
            }));
            edge == ScreenEdge::Top
        }
    }
}

// ---------------------------------------------------------------------------
// Esc

thread_local! {
    /// The global Esc monitor while the bubble or the Hands off pill shows,
    /// and which of them wants it.
    static ESCAPE_MONITOR: std::cell::RefCell<Option<Retained<AnyObject>>> =
        const { std::cell::RefCell::new(None) };
    static ESCAPE_WANTED: std::cell::Cell<(bool, bool)> =
        const { std::cell::Cell::new((false, false)) };
}

/// Notes whether the bubble and the pill want Esc, and watches for it while
/// either does.
fn want_escape(bubble: Option<bool>, hands: Option<bool>) {
    let (was_bubble, was_hands) = ESCAPE_WANTED.get();
    let wanted = (bubble.unwrap_or(was_bubble), hands.unwrap_or(was_hands));
    ESCAPE_WANTED.set(wanted);
    watch_escape(wanted.0 || wanted.1);
}

/// Watches for Esc in any app (a passive monitor: the key still reaches the
/// app). Esc stops the task moving the real pointer, unless that task
/// pressed it itself, and dismisses the bubble. Main thread only.
fn watch_escape(on: bool) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    ESCAPE_MONITOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        match (on, slot.take()) {
            (true, Some(monitor)) => *slot = Some(monitor),
            (true, None) => {
                let block = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| {
                    // SAFETY: AppKit passes a live event for the handler's duration.
                    let event = unsafe { event.as_ref() };
                    if event.keyCode() != ESCAPE {
                        return;
                    }
                    if let Some(task) = hands_task()
                        && pressed_by_person(event)
                    {
                        crate::assistant::cancel(&task);
                    }
                    if let Some(overlays) = OVERLAYS.get() {
                        overlays.bubble.update_if(|state, _| {
                            dismiss_bubble(state);
                            false
                        });
                    }
                });
                // AppKit retains the block; the monitor is removed on this
                // (main) thread when nothing wants it.
                *slot = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                    NSEventMask::KeyDown,
                    &block,
                );
            }
            (false, Some(monitor)) => {
                // SAFETY: the monitor came from addGlobalMonitor on this thread.
                unsafe { NSEvent::removeMonitor(&monitor) };
            }
            (false, None) => {}
        }
    });
}

/// `kCGEventSourceUnixProcessID`: the process that posted an event.
const SOURCE_PID: u32 = 41;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventGetIntegerValueField(event: *mut std::ffi::c_void, field: u32) -> i64;
}

/// Whether a key came from the person rather than from the agent pressing
/// keys for its task (agent-desktop, or Hex itself).
fn pressed_by_person(event: &NSEvent) -> bool {
    // SAFETY: `CGEvent` returns the event's own CGEventRef (or null), alive
    // while the event is; reading a field doesn't retain or change it.
    let pid = unsafe {
        let cg: *mut std::ffi::c_void = objc2::msg_send![event, CGEvent];
        if cg.is_null() {
            return true;
        }
        CGEventGetIntegerValueField(cg, SOURCE_PID)
    };
    if pid <= 0 {
        return true;
    }
    if pid == i64::from(std::process::id()) {
        return false;
    }
    let mut name = [0u8; 256];
    // SAFETY: `proc_name` writes at most `name.len()` bytes into the buffer.
    let written =
        unsafe { libc::proc_name(pid as i32, name.as_mut_ptr().cast(), name.len() as u32) };
    let name = std::str::from_utf8(&name[..written.max(0) as usize]).unwrap_or_default();
    !name.starts_with("agent-desktop")
}

// ---------------------------------------------------------------------------
// Native touches on the panels (main thread)

fn panel(number: Option<u32>) -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    let number = number? as isize;
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .find(|window| window.windowNumber() == number)
}

/// Hides the panel from Mission Control and Exposé (Transient) while it
/// still joins every Space and full-screen Space; Rune's own mask keeps it
/// in place there (Stationary) instead.
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
