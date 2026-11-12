//! The overlays on macOS: Rune panels fed by `assistant::subscribe`.
//!
//! - One frame panel per display (up to [`MAX_DISPLAYS`]), each the size of
//!   its display, shown while Agent mode is on. Core Animation breathes the
//!   panel's layer ([`ensure_breathing`]), so the frame never redraws.
//! - One outline panel per window a live task works in (up to
//!   [`MAX_OUTLINES`]), moved with its window by the watcher thread, which
//!   reads `computer::window_frame` ten times a second only while there are
//!   windows to outline, and the displays once a second while the frame is
//!   up.
//! - The task stack in a bottom corner of the pointer's display.
//! - The quick question's bubble, just past the dictation HUD.
//!
//! Every panel is click-through except the stack's plates and the bubble's
//! card, joins every Space and full-screen Space, is left out of screen
//! captures (so the agents' screenshots stay clean), and holds no window or
//! GPU surface while hidden. The service sends nothing while Labs ›
//! Assistant is off, so none of this ever shows then.

use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{NSNumber, NSString};
use rune::{
    Align, DisplayTarget, OverlayHandle, OverlayInput, OverlayOptions, OverlayPlacement,
    OverlaySpec, ScreenEdge, Tx,
};

use super::board::{Board, Bubble, Due, Outlined};
use super::bubble::{self, BubbleOverlay};
use super::frame::{self, Frame};
use super::hint::{self, Hint};
use super::outline::{self, Outline};
use super::stack::{self, Stack};
use crate::assistant::{ConversationId, Event, TaskWindow};
use crate::rune_ui::live_text::Spot;

/// Displays that get a frame; more are rare, and go without.
const MAX_DISPLAYS: usize = 4;
/// Windows outlined at once (the stack still lists every task).
const MAX_OUTLINES: usize = 8;
/// How the frame comes and goes.
const FRAME_IN: Duration = Duration::from_millis(700);
const FRAME_OUT: Duration = Duration::from_millis(500);
/// How the stack, outlines, and bubble come and go.
const FADE: Duration = Duration::from_millis(160);
/// How often outlined windows are found again while tasks run.
const OUTLINE_POLL: Duration = Duration::from_millis(100);
/// How often the displays are read while the frame is up.
const DISPLAY_POLL: Duration = Duration::from_secs(1);
/// Esc's key code.
const ESCAPE: u16 = 53;

struct Overlays {
    frames: Vec<OverlayHandle<Frame>>,
    outlines: Vec<OverlayHandle<Outline>>,
    stack: OverlayHandle<Stack>,
    bubble: OverlayHandle<BubbleOverlay>,
    hint: OverlayHandle<Hint>,
}

static OVERLAYS: OnceLock<Overlays> = OnceLock::new();

/// What the watcher thread watches, set from the subscriber and the
/// stack's state.
#[derive(Default)]
struct Watch {
    frame: bool,
    outlined: Vec<Outlined>,
    /// Bumped on every change, so the watcher wakes for it.
    generation: u64,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch {
    frame: false,
    outlined: Vec::new(),
    generation: 0,
});
static WATCH_CHANGED: Condvar = Condvar::new();
static WATCHER: OnceLock<()> = OnceLock::new();

/// Opens a conversation in Chats; set by the Chats pane ([`set_open_chats`]).
type OpenChats = Box<dyn Fn(&str) + Send + Sync>;
static OPEN_CHATS: OnceLock<OpenChats> = OnceLock::new();

fn watch() -> MutexGuard<'static, Watch> {
    WATCH.lock().unwrap_or_else(|error| error.into_inner())
}

/// Lets the Chats pane open a conversation when a stack plate or the
/// bubble's "Open in Chats" is pressed. Called on the main thread with the
/// conversation's id. Until it's set, a press brings the window forward.
#[allow(dead_code)] // Until Chats calls it.
pub(crate) fn set_open_chats(open: impl Fn(&str) + Send + Sync + 'static) {
    let _ = OPEN_CHATS.set(Box::new(open));
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

fn options(input: OverlayInput) -> OverlayOptions {
    OverlayOptions {
        input,
        exclude_from_capture: true,
        display: DisplayTarget::Primary,
        ..Default::default()
    }
}

/// Adds the overlays to the app and subscribes them to the assistant.
pub(crate) fn install<S: 'static>(app: &mut rune::App<S>) {
    let frames = (0..MAX_DISPLAYS)
        .map(|_| {
            let spec = OverlaySpec::new("Hex agent frame", (1440.0, 900.0), frame::view)
                .options(options(OverlayInput::None))
                .on_created(|created| leave_out_of_mission_control(created.window_number))
                .release_gpu_when_hidden();
            app.overlay(spec, Frame::default())
        })
        .collect();
    let outlines = (0..MAX_OUTLINES)
        .map(|_| {
            let spec = OverlaySpec::new("Hex agent window", (400.0, 300.0), outline::view)
                .options(options(OverlayInput::None))
                .on_created(|created| leave_out_of_mission_control(created.window_number))
                .release_gpu_when_hidden();
            app.overlay(spec, Outline::default())
        })
        .collect();
    let stack_spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Hex agent tasks", stack::SIZE, stack::view)
    }
    .options(options(OverlayInput::Regions))
    .poll(stack_poll)
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .animate_reveal()
    .release_gpu_when_hidden();
    let stack = app.overlay(
        stack_spec,
        Stack {
            board: Board::seeded(crate::assistant::tasks()),
            shown: false,
            armed: None,
            actions: stack::Actions {
                stop: |_, id| crate::assistant::cancel(id),
                open: |_, id| open_chats(id),
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
            actions: bubble::Actions {
                dismiss: dismiss_bubble,
                open: |state, id| {
                    open_chats(id);
                    dismiss_bubble(state);
                },
            },
        },
    );
    let hint = app.overlay(
        OverlaySpec::new("Hex new task", hint::SIZE, hint::view)
            .options(options(OverlayInput::None))
            .on_created(|created| leave_out_of_mission_control(created.window_number))
            .release_gpu_when_hidden(),
        Hint,
    );
    let _ = OVERLAYS.set(Overlays {
        frames,
        outlines,
        stack,
        bubble,
        hint,
    });
    // Tasks already running when Hex started listening.
    if let Some(overlays) = OVERLAYS.get() {
        overlays
            .stack
            .update(|stack, tx| board_changed(stack, tx, Instant::now()));
    }
    if crate::assistant::agent_mode() {
        set_frame(true);
    }
    crate::assistant::subscribe(Box::new(on_event));
    if std::env::var_os("HEX_ASSISTANT_FAKE").is_some() {
        play_fixture();
    }
}

/// `HEX_ASSISTANT_FAKE=1` (development, while the service is a stub): plays
/// `fixture::script` into the real overlays, with its windows laid out on
/// the main display, then finishes the tasks and turns Agent mode off.
static FAKE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn play_fixture() {
    FAKE.store(true, std::sync::atomic::Ordering::Release);
    let _ = std::thread::Builder::new()
        .name("hex-assistant-fake".into())
        .spawn(|| {
            let start = Instant::now();
            let mut script = super::fixture::script();
            let end = script.last().map_or(0.0, |(at, _)| *at);
            for (id, at) in [("table", 6.0), ("mail", 9.0)] {
                script.push((
                    end + at,
                    Event::Answer {
                        conversation: id.into(),
                        origin: crate::assistant::Origin::AgentMode,
                        text: "Done.".into(),
                    },
                ));
            }
            script.push((
                end + 11.0,
                Event::Cancelled {
                    conversation: "rename".into(),
                },
            ));
            script.push((end + 18.0, Event::ModeChanged { on: false }));
            for (at, event) in script {
                let due = start + Duration::from_secs_f64(at);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                on_event(&event);
            }
        });
}

/// Where a task's window is: Accessibility's answer, or the fixture's
/// layout while it plays.
fn window_frame(window: &TaskWindow, displays: &[Display]) -> Option<(f64, f64, f64, f64)> {
    if FAKE.load(std::sync::atomic::Ordering::Acquire) {
        let main = displays.first()?.bounds;
        let (x, y, w, h) = super::fixture::frame(window, (main[2], main[3]))?;
        return Some((main[0] + x, main[1] + y, w, h));
    }
    crate::assistant::computer::window_frame(window)
}

/// The subscriber: on the service's thread, so it only queues work for the
/// overlays' (main) thread and the watcher.
fn on_event(event: &Event) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    match event {
        Event::ModeChanged { on } => set_frame(*on),
        Event::ConversationsChanged => {}
        _ => {
            let for_stack = event.clone();
            overlays
                .stack
                .update_if(move |stack, tx| stack_event(stack, &for_stack, tx));
            let for_bubble = event.clone();
            overlays
                .bubble
                .update_if(move |state, tx| bubble_event(state, &for_bubble, tx));
        }
    }
}

// ---------------------------------------------------------------------------
// The stack (and the board the outlines read)

fn stack_event(stack: &mut Stack, event: &Event, tx: &mut Tx) -> bool {
    let colour = match event {
        Event::Queued { conversation, .. } | Event::Started { conversation, .. } => {
            colour_of(conversation)
        }
        _ => None,
    };
    let now = Instant::now();
    if !stack.board.apply(event, colour, now) {
        return false;
    }
    board_changed(stack, tx, now);
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

fn board_changed(stack: &mut Stack, tx: &mut Tx, now: Instant) {
    publish_outlines(stack.board.outlined());
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let visible = !stack.board.stack().is_empty();
    if visible && !stack.shown {
        place_stack(&overlays.stack);
        overlays.stack.fade_in(FADE);
        stack.shown = true;
    } else if !visible && stack.shown {
        overlays.stack.fade_out(FADE);
        stack.shown = false;
    }
    arm(stack, tx, now);
}

/// Wakes the stack when its next finished task is due to leave.
fn arm(stack: &mut Stack, tx: &mut Tx, now: Instant) {
    let next = stack.board.next_expiry();
    if let Some(at) = next
        && next != stack.armed
    {
        tx.wake_after(at.saturating_duration_since(now));
    }
    stack.armed = next;
}

fn stack_poll(stack: &mut Stack, _clock: f64, tx: &mut Tx) -> bool {
    let now = Instant::now();
    if stack.board.expire(now) {
        stack.armed = None;
        board_changed(stack, tx, now);
        return true;
    }
    if stack.armed.is_some_and(|at| now >= at) {
        // Woken a little early: look again.
        stack.armed = None;
        arm(stack, tx, now);
    }
    false
}

/// Puts the stack in its corner of the pointer's display, inside the area
/// the menu bar and Dock leave.
fn place_stack(handle: &OverlayHandle<Stack>) {
    let Some((screen, primary_top, _, _)) =
        MainThreadMarker::new().and_then(crate::rune_hud::pointer_screen)
    else {
        return;
    };
    let visible = screen.visible;
    let global = [
        visible.x,
        primary_top - (visible.y + visible.height),
        visible.width,
        visible.height,
    ];
    let corner = stack::corner(crate::app_settings::meeting_indicator_corner());
    let [x, y, width, height] = stack::rect(global, corner);
    handle.set_display(DisplayTarget::Primary);
    handle.set_placement(Some(OverlayPlacement::Rect {
        x,
        y,
        width,
        height,
    }));
}

// ---------------------------------------------------------------------------
// "New task" beside the HUD

/// Whether the "New task" hint shows.
static HINT_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Shows "New task" just past the HUD while `on` (the Action shortcut held
/// in Agent mode), and hides it after. The HUD calls it on the main thread
/// with each event, once it has placed itself; a repeat does nothing.
pub(crate) fn new_task_hint(on: bool) {
    use std::sync::atomic::Ordering;
    if HINT_SHOWN.swap(on, Ordering::AcqRel) == on {
        return;
    }
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    if on {
        place_hint(&overlays.hint);
        overlays.hint.fade_in(FADE);
    } else {
        overlays.hint.fade_out(FADE);
    }
}

/// Places the hint where Live text would start, past its plate when Labs ›
/// Live text may show one there.
fn place_hint(handle: &OverlayHandle<Hint>) {
    let (width, height) = hint::SIZE;
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
            // Cancelled: it fades with what it showed.
            state.bubble = before;
            hide_bubble(state);
        }
        Some(bubble) => {
            let fresh = before
                .as_ref()
                .is_none_or(|old| old.conversation != bubble.conversation);
            if fresh || !state.shown {
                state.below = place_bubble(&overlays.bubble);
                overlays.bubble.fade_in(FADE);
                state.shown = true;
                watch_escape(true);
            }
            settle_bubble(state, tx, now);
        }
    }
    true
}

fn bubble_poll(state: &mut BubbleOverlay, _clock: f64, tx: &mut Tx) -> bool {
    settle_bubble(state, tx, Instant::now())
}

/// Hides an answered bubble once its time is up; whether it changed.
fn settle_bubble(state: &mut BubbleOverlay, tx: &mut Tx, now: Instant) -> bool {
    if !state.shown {
        return false;
    }
    let Some(bubble) = state.bubble.as_mut() else {
        return false;
    };
    match bubble.due(now, crate::assistant::speech::speaking()) {
        Due::Stay => false,
        Due::At(at) => {
            tx.wake_after(at.saturating_duration_since(now));
            false
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
    watch_escape(false);
    if let Some(overlays) = OVERLAYS.get() {
        overlays.bubble.fade_out(FADE);
    }
}

/// A press on the card, or Esc: the bubble goes and stops reading aloud.
/// The question itself goes on; its answer is in Chats.
fn dismiss_bubble(state: &mut BubbleOverlay) {
    if state.shown {
        crate::assistant::speech::stop();
    }
    hide_bubble(state);
}

/// Places the bubble where Live text would show, just past the HUD's plate;
/// whether it hangs below that spot.
fn place_bubble(handle: &OverlayHandle<BubbleOverlay>) -> bool {
    let (width, height) = bubble::SIZE;
    match crate::rune_hud::live_text_place() {
        Spot::Global { x, y, below } => {
            handle.set_display(DisplayTarget::Primary);
            handle.set_placement(Some(OverlayPlacement::Rect {
                x: x - width / 2.0,
                y: if below {
                    y - bubble::INSET
                } else {
                    y - height + bubble::INSET
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
                margin: margin - bubble::INSET,
                align: Align::Center,
                inset: 0.0,
            }));
            edge == ScreenEdge::Top
        }
    }
}

thread_local! {
    /// The global Esc monitor while the bubble shows.
    static ESCAPE_MONITOR: std::cell::RefCell<Option<Retained<AnyObject>>> =
        const { std::cell::RefCell::new(None) };
}

/// Watches for Esc in any app while the bubble shows (a passive monitor:
/// the key still reaches the app). Main thread only.
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
                    if unsafe { event.as_ref() }.keyCode() == ESCAPE
                        && let Some(overlays) = OVERLAYS.get()
                    {
                        overlays.bubble.update_if(|state, _| {
                            dismiss_bubble(state);
                            false
                        });
                    }
                });
                // AppKit retains the block; the monitor is removed on this
                // (main) thread when the bubble hides.
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

// ---------------------------------------------------------------------------
// The frame and outlines: the watcher thread

fn set_frame(on: bool) {
    let mut watch = watch();
    if watch.frame == on {
        return;
    }
    watch.frame = on;
    watch.generation += 1;
    drop(watch);
    start_watcher();
    WATCH_CHANGED.notify_all();
}

fn publish_outlines(outlined: Vec<Outlined>) {
    let mut watch = watch();
    if watch.outlined == outlined {
        return;
    }
    watch.outlined = outlined;
    watch.generation += 1;
    drop(watch);
    start_watcher();
    WATCH_CHANGED.notify_all();
}

/// Starts the watcher on first use, so a Hex that never uses the assistant
/// runs no thread for it.
fn start_watcher() {
    WATCHER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("hex-assistant-overlays".into())
            .spawn(|| Watcher::default().run());
    });
}

/// A display's id (`CGDirectDisplayID`, the `NSScreenNumber` Rune pins
/// overlays by) and bounds in global points, y down.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Display {
    id: u32,
    bounds: [f64; 4],
}

/// What one outline panel shows.
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    key: (ConversationId, TaskWindow),
    rect: Option<[f32; 4]>,
    outline: Outline,
    /// The window's CoreGraphics number, when its id names one: the
    /// outline then sits among ordinary windows, just above it.
    target: Option<u32>,
    /// The panel's level was set for `target`.
    levelled: bool,
}

#[derive(Default)]
struct Watcher {
    /// The display each frame panel covers.
    frames: Vec<Option<Display>>,
    displays_read: Option<Instant>,
    displays: Vec<Display>,
    slots: Vec<Option<Slot>>,
}

impl Watcher {
    fn run(mut self) {
        self.frames = vec![None; MAX_DISPLAYS];
        self.slots = vec![None; MAX_OUTLINES];
        let mut seen = u64::MAX;
        loop {
            let (frame, outlined) = {
                let mut watch = watch();
                // Sleep while there's nothing to show or move.
                while !watch.frame
                    && watch.outlined.is_empty()
                    && watch.generation == seen
                    && self.idle()
                {
                    watch = WATCH_CHANGED
                        .wait(watch)
                        .unwrap_or_else(|error| error.into_inner());
                }
                seen = watch.generation;
                (watch.frame, watch.outlined.clone())
            };
            self.frames(frame);
            self.outlines(&outlined);
            let pause = if !outlined.is_empty() {
                OUTLINE_POLL
            } else if frame {
                DISPLAY_POLL
            } else {
                continue;
            };
            let watch = watch();
            if watch.generation == seen {
                let _ = WATCH_CHANGED.wait_timeout(watch, pause);
            }
        }
    }

    /// Nothing is on screen.
    fn idle(&self) -> bool {
        self.frames.iter().all(Option::is_none) && self.slots.iter().all(Option::is_none)
    }

    /// Reads the displays when a second has passed since the last read;
    /// whether it did.
    fn read_displays(&mut self) -> bool {
        let due = self
            .displays_read
            .is_none_or(|at| at.elapsed() >= DISPLAY_POLL);
        if due {
            self.displays = displays();
            self.displays_read = Some(Instant::now());
        }
        due
    }

    /// Shows a frame on each display while Agent mode is on, follows
    /// displays coming, going, and resizing, and keeps each breathing.
    fn frames(&mut self, on: bool) {
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        let read = on && self.read_displays();
        for (index, handle) in overlays.frames.iter().enumerate() {
            let want = on.then(|| self.displays.get(index).copied()).flatten();
            let have = self.frames[index];
            match (have, want) {
                (_, Some(display)) if have != want => {
                    let [x, y, width, height] = display.bounds.map(|v| v as f32);
                    handle.update(move |frame, _| frame.display = Some(display.id));
                    handle.set_display(DisplayTarget::Primary);
                    handle.set_placement(Some(OverlayPlacement::Rect {
                        x,
                        y,
                        width,
                        height,
                    }));
                    if have.is_none() {
                        handle.fade_in(FRAME_IN);
                    }
                }
                (Some(_), None) => handle.fade_out(FRAME_OUT),
                _ => {}
            }
            let moved = have != want;
            self.frames[index] = want;
            // Checked with each read of the displays, not each outline poll.
            if want.is_some() && (read || moved) {
                let breathing = handle.clone();
                // On the main thread; no redraw.
                handle.update_if(move |_, _| {
                    ensure_breathing(breathing.window_number());
                    false
                });
            }
        }
    }

    /// Moves each live task window's outline to where its window is now.
    fn outlines(&mut self, outlined: &[Outlined]) {
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        if !outlined.is_empty() {
            self.read_displays();
        }
        let keys: Vec<_> = outlined
            .iter()
            .map(|o| (o.task.clone(), o.window.clone()))
            .collect();
        // Free the slots of windows no longer outlined.
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|slot| !keys.contains(&slot.key)) {
                if slot.as_ref().is_some_and(|slot| slot.rect.is_some()) {
                    overlays.outlines[index].fade_out(FADE);
                }
                *slot = None;
            }
        }
        for item in outlined {
            let key = (item.task.clone(), item.window.clone());
            let index = match self
                .slots
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|slot| slot.key == key))
            {
                Some(index) => index,
                None => match self.slots.iter().position(Option::is_none) {
                    Some(free) => {
                        self.slots[free] = Some(Slot {
                            key,
                            rect: None,
                            outline: Outline::default(),
                            target: None,
                            levelled: false,
                        });
                        free
                    }
                    // Every panel is taken; this window goes without.
                    None => continue,
                },
            };
            let handle = &overlays.outlines[index];
            let Some(slot) = self.slots[index].as_mut() else {
                continue;
            };
            let found = window_frame(&item.window, &self.displays)
                .filter(|(_, _, w, h)| *w >= 1.0 && *h >= 1.0);
            let Some(window) = found else {
                if slot.rect.take().is_some() {
                    handle.fade_out(FADE);
                }
                continue;
            };
            let top = display_top(&self.displays, window);
            let (rect, label) = outline::rect(window, top);
            let drawn = Outline {
                colour: item.colour,
                title: item.title.clone(),
                size: (window.2 as f32, window.3 as f32),
                label,
            };
            if drawn != slot.outline {
                slot.outline = drawn.clone();
                handle.update(move |outline, _| *outline = drawn);
            }
            if slot.rect != Some(rect) {
                let [x, y, width, height] = rect;
                handle.set_display(DisplayTarget::Primary);
                handle.set_placement(Some(OverlayPlacement::Rect {
                    x,
                    y,
                    width,
                    height,
                }));
                if slot.rect.is_none() {
                    handle.fade_in(FADE);
                }
                slot.rect = Some(rect);
            }
            let target = item
                .window
                .window_id
                .as_deref()
                .and_then(crate::assistant::computer::window_number);
            if !slot.levelled || slot.target != target {
                // Among ordinary windows when its window is known, so a
                // window covering it covers its outline too; otherwise
                // over everything, like the other overlays.
                handle.set_macos_level(if target.is_some() {
                    NORMAL_LEVEL
                } else {
                    STATUS_LEVEL
                });
                slot.target = target;
                slot.levelled = true;
            }
            if let Some(target) = target {
                // Each poll, since clicking another window reorders them.
                let panel = handle.clone();
                handle.update_if(move |_, _| {
                    order_above(panel.window_number(), target);
                    false
                });
            }
        }
    }
}

/// `NSNormalWindowLevel`: ordinary app windows.
const NORMAL_LEVEL: isize = 0;
/// `NSStatusWindowLevel`, Rune's default for overlays.
const STATUS_LEVEL: isize = 25;

/// Orders the outline panel `number` just above window `target` (another
/// app's, by its CoreGraphics number), when it shows. Main thread.
fn order_above(number: Option<u32>, target: u32) {
    let Some(panel) = panel(number) else {
        return;
    };
    if !panel.isVisible() {
        return;
    }
    panel.orderWindow_relativeTo(objc2_app_kit::NSWindowOrderingMode::Above, target as isize);
}

/// The top of the display the window's top edge is on (its centre's, else
/// the nearest display's).
fn display_top(displays: &[Display], (x, y, w, _): (f64, f64, f64, f64)) -> f64 {
    let centre = x + w / 2.0;
    displays
        .iter()
        .find(|d| {
            let [dx, dy, dw, dh] = d.bounds;
            centre >= dx && centre < dx + dw && y >= dy && y < dy + dh
        })
        .or_else(|| {
            displays.iter().min_by(|a, b| {
                let distance = |d: &&Display| (d.bounds[0] + d.bounds[2] / 2.0 - centre).abs();
                distance(a).total_cmp(&distance(b))
            })
        })
        .map_or(0.0, |d| d.bounds[1])
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CgRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CgRect;
}

/// The active displays, the main one first, in global points (y down from
/// the main display's top: Rune's global space and Accessibility's).
fn displays() -> Vec<Display> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // SAFETY: the buffer holds `ids.len()` entries and CoreGraphics writes
    // at most that many; these calls are thread-safe.
    let status = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if status != 0 {
        return Vec::new();
    }
    ids[..count as usize]
        .iter()
        .map(|&id| {
            // SAFETY: a display id CoreGraphics just listed.
            let r = unsafe { CGDisplayBounds(id) };
            Display {
                id,
                bounds: [r.x, r.y, r.width, r.height],
            }
        })
        .collect()
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

/// Whether macOS asks for less motion (Accessibility › Display › Reduce
/// motion).
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

const BREATHE_KEY: &str = "hex-breathe";

/// Breathes the frame panel's layer in Core Animation (none with Reduce
/// motion on): the window server animates it, so Hex draws nothing for it.
/// Added again if the layer was remade.
fn ensure_breathing(number: Option<u32>) {
    let Some(panel) = panel(number) else {
        return;
    };
    let Some(view) = panel.contentView() else {
        return;
    };
    let key = NSString::from_str(BREATHE_KEY);
    // SAFETY: Core Animation calls on the main thread, on the panel's own
    // layer, with objects that live through each call.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        let Some(layer) = layer.as_ref() else {
            return;
        };
        let existing: *mut AnyObject = msg_send![layer, animationForKey: &*key];
        if reduce_motion() {
            if !existing.is_null() {
                let _: () = msg_send![layer, removeAnimationForKey: &*key];
            }
            return;
        }
        if !existing.is_null() {
            return;
        }
        let (Some(animation_class), Some(timing_class)) = (
            AnyClass::get(c"CABasicAnimation"),
            AnyClass::get(c"CAMediaTimingFunction"),
        ) else {
            return;
        };
        let path = NSString::from_str("opacity");
        let animation: Option<Retained<AnyObject>> =
            msg_send![animation_class, animationWithKeyPath: &*path];
        let Some(animation) = animation else {
            return;
        };
        let from = NSNumber::new_f32(1.0);
        let to = NSNumber::new_f32(frame::BREATH_LOW);
        let ease = NSString::from_str("easeInEaseOut");
        let timing: Option<Retained<AnyObject>> = msg_send![timing_class, functionWithName: &*ease];
        let _: () = msg_send![&*animation, setFromValue: &*from];
        let _: () = msg_send![&*animation, setToValue: &*to];
        let _: () = msg_send![&*animation, setDuration: frame::BREATH / 2.0];
        let _: () = msg_send![&*animation, setAutoreverses: true];
        let _: () = msg_send![&*animation, setRepeatCount: f32::INFINITY];
        let _: () = msg_send![&*animation, setRemovedOnCompletion: false];
        if let Some(timing) = &timing {
            let _: () = msg_send![&*animation, setTimingFunction: &**timing];
        }
        let _: () = msg_send![layer, addAnimation: &*animation, forKey: &*key];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outlines_label_measures_from_its_own_displays_top() {
        let displays = [
            Display {
                id: 1,
                bounds: [0.0, 0.0, 1512.0, 982.0],
            },
            Display {
                id: 2,
                bounds: [-1000.0, -1080.0, 1920.0, 1080.0],
            },
        ];
        assert_eq!(display_top(&displays, (100.0, 300.0, 600.0, 400.0)), 0.0);
        assert_eq!(
            display_top(&displays, (100.0, -900.0, 600.0, 400.0)),
            -1080.0
        );
        // Off every display: the nearest one's.
        assert_eq!(display_top(&displays, (5000.0, 300.0, 600.0, 400.0)), 0.0);
    }
}
