//! The watcher thread: shows the desk frame on every display while the
//! agent has the person's controls (real input, or a window brought in
//! front), with its tab on the display it's about and "Back to you" after;
//! moves each outline with its window (only windows on the screen: a
//! parked one shows as its mini's ring in the rail instead); keeps the
//! outlined windows' rects for the rail's flip rule (`place::anchor_clear`);
//! and, while a task might hold the real pointer, asks the guard who does.
//! It sleeps while there's nothing to show, move, or ask.

use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use rune::{DisplayTarget, OverlayPlacement};

use super::super::board::Outlined;
use super::super::frame::{Frame, Tab};
use super::super::outline::{self, LabelSide, Outline};
use super::super::screens::{self, Screen, Seams};
#[cfg(target_os = "macos")]
use super::panel;
use super::{FADE, OVERLAYS, displays};
use crate::assistant::{ConversationId, Event, TaskWindow};

/// Frame panels: one per display, as many as macOS drives at once.
pub(super) const MAX_DISPLAYS: usize = 16;
/// Outline panels: a window spanning two displays takes one per display
/// (the list still shows every task).
pub(super) const MAX_OUTLINES: usize = 16;
/// How the frame comes and goes.
const FRAME_IN: Duration = Duration::from_millis(700);
const FRAME_OUT: Duration = Duration::from_millis(500);
/// How often outlined windows are found again while tasks run.
const OUTLINE_POLL: Duration = Duration::from_millis(100);
/// How often the displays are read while the frame is up, and the guard
/// asked who holds the pointer while a task runs.
const DISPLAY_POLL: Duration = Duration::from_secs(1);

/// Of a window, at least this much on the displays for its outline: less
/// is a parked window hanging off the edge (`space::off_screen`).
const ON_SCREEN: f64 = 0.5;
/// The shortest wait while "Back to you" counts down.
const TAB_TICK: Duration = Duration::from_millis(50);

/// What the watcher thread watches, set from the subscriber and the orb.
#[derive(Default)]
struct Watch {
    /// The desk frame shows until then (`Board::frame_until`).
    frame: Option<Instant>,
    /// The frame's tab (or "Back to you" after it).
    tab: Option<Tab>,
    outlined: Vec<Outlined>,
    /// A task is live, so one might take the real pointer.
    live: bool,
    /// Bumped on every change, so the watcher wakes for it.
    generation: u64,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch {
    frame: None,
    tab: None,
    outlined: Vec::new(),
    live: false,
    generation: 0,
});
static WATCH_CHANGED: Condvar = Condvar::new();
static WATCHER: OnceLock<()> = OnceLock::new();

fn watch() -> MutexGuard<'static, Watch> {
    WATCH.lock().unwrap_or_else(|error| error.into_inner())
}

/// Shows the desk frame until `until` (`None` hides it), and its `tab`.
pub(super) fn set_frame(until: Option<Instant>, tab: Option<Tab>) {
    change(|watch| {
        let changed = watch.frame != until || watch.tab != tab;
        watch.frame = until;
        watch.tab = tab;
        changed
    });
}

/// The windows outlined now (global points): the rail flips away from them.
static OUTLINED_AT: Mutex<Vec<[f64; 4]>> = Mutex::new(Vec::new());

/// The windows outlined now, in global points.
pub(super) fn outlined_rects() -> Vec<[f64; 4]> {
    OUTLINED_AT
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// The windows to outline, and whether any task is live.
pub(super) fn publish(outlined: Vec<Outlined>, live: bool) {
    change(|watch| {
        let changed = watch.outlined != outlined || watch.live != live;
        watch.outlined = outlined;
        watch.live = live;
        changed
    });
}

/// Displays came, went, or changed: look again now.
pub(super) fn displays_changed() {
    change(|_| true);
}

fn change(apply: impl FnOnce(&mut Watch) -> bool) {
    let mut watch = watch();
    if !apply(&mut watch) {
        return;
    }
    watch.generation += 1;
    drop(watch);
    start();
    WATCH_CHANGED.notify_all();
}

/// Starts the watcher on first use, so an Agent-Desk that never uses the assistant
/// runs no thread for it.
fn start() {
    WATCHER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("hex-assistant-overlays".into())
            .spawn(|| Watcher::default().run());
    });
}

/// One outline piece to draw: its task window, the display it's on, the
/// whole outline's rect and this piece's (global points), where the label
/// goes, and the window's size.
type Piece<'a> = (&'a Outlined, u32, [f64; 4], [f64; 4], LabelSide, (f64, f64));

/// What one frame panel shows: its display, how it meets the others,
/// whether it glows, and its tab with the seconds left.
#[derive(Clone, Debug, PartialEq)]
struct FrameSlot {
    screen: Screen,
    seams: Seams,
    glow: bool,
    tab: Option<Tab>,
    left: Option<u64>,
}

/// What one outline panel shows: one task window's outline, or its piece
/// on one display.
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    key: (ConversationId, TaskWindow, u32),
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
    frames: Vec<Option<FrameSlot>>,
    displays_read: Option<Instant>,
    displays: Vec<Screen>,
    slots: Vec<Option<Slot>>,
    /// When the guard was last asked, and who held the pointer then.
    asked: Option<Instant>,
    hands: Option<ConversationId>,
    /// When the HUD-side panels were last asked to follow the person.
    followed: Option<Instant>,
    /// The display the tab last showed on ("Back to you" stays there).
    tab_on: Option<u32>,
}

impl Watcher {
    fn run(mut self) {
        self.frames = vec![None; MAX_DISPLAYS];
        self.slots = vec![None; MAX_OUTLINES];
        let mut seen = u64::MAX;
        loop {
            let (frame, tab, outlined, live) = {
                let mut watch = watch();
                // Sleep while there's nothing to show, move, or ask.
                while watch.frame.is_none_or(|until| until <= Instant::now())
                    && watch
                        .tab
                        .as_ref()
                        .is_none_or(|tab| !tab.shows(Instant::now()))
                    && watch.outlined.is_empty()
                    && !watch.live
                    && watch.generation == seen
                    && self.idle()
                {
                    watch = WATCH_CHANGED
                        .wait(watch)
                        .unwrap_or_else(|error| error.into_inner());
                }
                seen = watch.generation;
                let now = Instant::now();
                (
                    watch.frame.is_some_and(|until| until > now),
                    watch.tab.clone().filter(|tab| tab.shows(now)),
                    watch.outlined.clone(),
                    watch.live,
                )
            };
            if displays::take_changed() {
                // Read them again now, not within the second.
                self.displays_read = None;
            }
            self.frames(frame, tab.as_ref());
            self.outlines(&outlined, frame);
            self.ask_guard(live);
            if (frame || live) && self.followed.is_none_or(|at| at.elapsed() >= DISPLAY_POLL) {
                self.followed = Some(Instant::now());
                // The orb and bubble move to the display the person works on.
                super::follow::follow(false);
            }
            let back = match &tab {
                Some(Tab::Back { until, .. }) => Some(
                    until
                        .saturating_duration_since(Instant::now())
                        .max(TAB_TICK),
                ),
                _ => None,
            };
            let pause = if !outlined.is_empty() {
                OUTLINE_POLL
            } else if frame || live || tab.is_some() {
                DISPLAY_POLL
            } else {
                continue;
            };
            let pause = back.map_or(pause, |back| back.min(pause));
            let watch = watch();
            if watch.generation == seen {
                let _ = WATCH_CHANGED.wait_timeout(watch, pause);
            }
        }
    }

    /// Nothing is on screen and nobody holds the pointer.
    fn idle(&self) -> bool {
        self.frames.iter().all(Option::is_none)
            && self.slots.iter().all(Option::is_none)
            && self.hands.is_none()
    }

    /// While a task is live, asks the guard once a second who moves the real
    /// pointer, and tells the overlays when that changes (as the service's
    /// `HandsOff` would).
    fn ask_guard(&mut self, live: bool) {
        if !live && self.hands.is_none() {
            return;
        }
        if self.asked.is_some_and(|at| at.elapsed() < DISPLAY_POLL) {
            return;
        }
        self.asked = Some(Instant::now());
        let now = if live {
            crate::assistant::guard::hands_off()
        } else {
            None
        };
        if now == self.hands {
            return;
        }
        if let Some(before) = self.hands.take() {
            super::on_event(&Event::HandsOff {
                conversation: before,
                on: false,
            });
        }
        if let Some(task) = &now {
            super::on_event(&Event::HandsOff {
                conversation: task.clone(),
                on: true,
            });
        }
        self.hands = now;
    }

    /// Reads the displays when a second has passed since the last read (or
    /// they changed); whether it did.
    fn read_displays(&mut self) -> bool {
        let due = self
            .displays_read
            .is_none_or(|at| at.elapsed() >= DISPLAY_POLL);
        if due {
            self.displays = displays::read();
            self.displays_read = Some(Instant::now());
        }
        due
    }

    /// Shows the desk frame on every display while the agent has the
    /// controls (`glow`), each joined to its neighbours, with the `tab` on
    /// the display it's about (its window's, else where it showed last,
    /// else the main one); "Back to you" alone keeps only that display's
    /// panel. Follows displays coming, going, moving, and resizing (a panel
    /// stays with its display). The frame is still: nothing breathes while
    /// the agent is in front.
    fn frames(&mut self, glow: bool, tab: Option<&Tab>) {
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        if glow || tab.is_some() {
            self.read_displays();
        }
        let tab_on = tab.and_then(|tab| self.tab_display(tab));
        self.tab_on = tab_on.or(self.tab_on);
        let left = tab.and_then(|tab| tab.left(super::super::when::now_ms()));
        let wanted: Vec<FrameSlot> = self
            .displays
            .iter()
            .filter(|screen| glow || Some(screen.id) == tab_on)
            .map(|screen| {
                let here = Some(screen.id) == tab_on;
                FrameSlot {
                    screen: screen.clone(),
                    seams: screens::seams(screen, &self.displays),
                    glow,
                    tab: tab.filter(|_| here).cloned(),
                    left: left.filter(|_| here),
                }
            })
            .collect();
        // Displays that went (or no longer show it): their panels go.
        for (index, slot) in self.frames.iter_mut().enumerate() {
            let gone = slot
                .as_ref()
                .is_some_and(|have| !wanted.iter().any(|want| want.screen.id == have.screen.id));
            if gone {
                overlays.frames[index].fade_out(FRAME_OUT);
                *slot = None;
            }
        }
        for want in wanted {
            let index = match self.frames.iter().position(|slot| {
                slot.as_ref()
                    .is_some_and(|have| have.screen.id == want.screen.id)
            }) {
                Some(index) => index,
                None => match self.frames.iter().position(Option::is_none) {
                    Some(free) => free,
                    None => continue,
                },
            };
            let handle = &overlays.frames[index];
            let have = self.frames[index].clone();
            if have.as_ref() != Some(&want) {
                let [x, y, width, height] = want.screen.frame.map(|v| v as f32);
                let state = Frame {
                    display: Some(want.screen.id),
                    size: (width, height),
                    seams: want.seams.clone(),
                    glow: want.glow,
                    tab: want.tab.clone(),
                    left: want.left,
                };
                handle.update(move |frame, _| *frame = state);
                let moved = have
                    .as_ref()
                    .is_none_or(|have| have.screen.frame != want.screen.frame);
                if moved {
                    handle.set_display(DisplayTarget::Primary);
                    handle.set_placement(Some(OverlayPlacement::Rect {
                        x,
                        y,
                        width,
                        height,
                    }));
                }
                if have.is_none() {
                    handle.fade_in(FRAME_IN);
                }
            }
            self.frames[index] = Some(want);
        }
    }

    /// The display the tab goes on: its window's, else the one it showed
    /// on last, else the main display.
    fn tab_display(&self, tab: &Tab) -> Option<u32> {
        let window = tab
            .window()
            .and_then(|window| super::window_frame(window, &self.displays))
            .map(|(x, y, w, h)| [x, y, w, h]);
        window
            .and_then(|rect| screens::under(&self.displays, rect))
            .map(|screen| screen.id)
            .or(self
                .tab_on
                .filter(|id| self.displays.iter().any(|screen| screen.id == *id)))
            .or(self.displays.first().map(|screen| screen.id))
    }

    /// Moves each live task window's outline to where its window is now: in
    /// one piece per display it spans, and none for a window on no display
    /// (one left where an unplugged display was).
    fn outlines(&mut self, outlined: &[Outlined], frame_on: bool) {
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        if !outlined.is_empty() {
            self.read_displays();
        }
        // Each outline's pieces now: one outline per window, none for a
        // window the screen frame already runs along (`outline::redundant`).
        let mut wanted: Vec<Piece<'_>> = Vec::new();
        let mut drawn: Vec<[f64; 4]> = Vec::new();
        let screens: Vec<[f64; 4]> = self.displays.iter().map(|screen| screen.frame).collect();
        for item in outlined {
            let Some(window) = super::window_frame(&item.window, &self.displays)
                .filter(|(_, _, w, h)| *w >= 1.0 && *h >= 1.0)
            else {
                continue;
            };
            let window_rect = [window.0, window.1, window.2, window.3];
            if !on_screen(window_rect, &screens) {
                // Parked off the screen: its mini's ring says it instead.
                continue;
            }
            if outline::redundant(window_rect, &drawn, &screens, frame_on) {
                continue;
            }
            drawn.push(window_rect);
            let top = display_top(&self.displays, window);
            let (rect, label) = outline::rect(window, top);
            let full = rect.map(f64::from);
            for (display, piece) in screens::pieces(full, &self.displays) {
                wanted.push((item, display, full, piece, label, (window.2, window.3)));
            }
        }
        *OUTLINED_AT
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = drawn.clone();
        let keys: Vec<_> = wanted
            .iter()
            .map(|(item, display, ..)| (item.task.clone(), item.window.clone(), *display))
            .collect();
        // Free the panels of outlines (and pieces) no longer drawn.
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|slot| !keys.contains(&slot.key)) {
                if slot.as_ref().is_some_and(|slot| slot.rect.is_some()) {
                    overlays.outlines[index].fade_out(FADE);
                }
                *slot = None;
            }
        }
        for ((item, _, full, piece, label, size), key) in wanted.into_iter().zip(keys) {
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
                    // Every panel is taken; this piece goes without.
                    None => continue,
                },
            };
            let handle = &overlays.outlines[index];
            let Some(slot) = self.slots[index].as_mut() else {
                continue;
            };
            let drawn = Outline {
                colour: item.colour,
                title: item.title.clone(),
                size: (size.0 as f32, size.1 as f32),
                label,
                look: item.look,
                progress: item.progress,
                step: item.step.clone(),
                here: item.here,
                origin: ((full[0] - piece[0]) as f32, (full[1] - piece[1]) as f32),
            };
            if drawn != slot.outline {
                slot.outline = drawn.clone();
                handle.update(move |outline, _| *outline = drawn);
            }
            let rect = piece.map(|v| v as f32);
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

// ---------------------------------------------------------------------------
// Esc while a window is in front

/// `kVK_Escape`.
#[cfg(target_os = "macos")]
const ESCAPE: u16 = 53;

/// The task whose window is in front, while the tab shows: Esc stops it.
static IN_FRONT: Mutex<Option<ConversationId>> = Mutex::new(None);

#[cfg(target_os = "macos")]
thread_local! {
    /// The global Esc monitor while a window is borrowed.
    static IN_FRONT_MONITOR: std::cell::RefCell<Option<objc2::rc::Retained<objc2::runtime::AnyObject>>> =
        const { std::cell::RefCell::new(None) };
}

/// Esc (pressed by the person, in any app) stops `task` while its window
/// is in front, as the tab says; `None` stops watching. Main thread.
pub(super) fn escape_stops(task: Option<ConversationId>) {
    let on = task.is_some();
    *IN_FRONT.lock().unwrap_or_else(|error| error.into_inner()) = task;
    watch_in_front_escape(on);
}

/// Linux and Windows: the listener takes Escape while the assistant is busy
/// (`assistant::shortcut::claims_escape`).
#[cfg(not(target_os = "macos"))]
fn watch_in_front_escape(_on: bool) {}

/// A passive monitor (the key still reaches the app in front).
#[cfg(target_os = "macos")]
fn watch_in_front_escape(on: bool) {
    use objc2_app_kit::{NSEvent, NSEventMask};
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    IN_FRONT_MONITOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        match (on, slot.take()) {
            (true, Some(monitor)) => *slot = Some(monitor),
            (true, None) => {
                let block = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| {
                    // SAFETY: AppKit passes a live event for the handler's duration.
                    let event = unsafe { event.as_ref() };
                    if event.keyCode() != ESCAPE || !super::answer::pressed_by_person(event) {
                        return;
                    }
                    let task = IN_FRONT
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone();
                    if let Some(task) = task {
                        crate::assistant::cancel(&task);
                    }
                });
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

/// `NSNormalWindowLevel`: ordinary app windows.
const NORMAL_LEVEL: isize = 0;
/// `NSStatusWindowLevel`, Rune's default for overlays.
const STATUS_LEVEL: isize = 25;

/// Linux and Windows draw outlines above everything.
#[cfg(not(target_os = "macos"))]
fn order_above(_number: Option<u32>, _target: u32) {}

/// Orders the outline panel `number` just above window `target` (another
/// app's, by its CoreGraphics number), when it shows. Main thread.
#[cfg(target_os = "macos")]
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
/// the nearest display's), so its label stays on that display.
fn display_top(displays: &[Screen], (x, y, w, _): (f64, f64, f64, f64)) -> f64 {
    screens::at(displays, (x + w / 2.0, y)).map_or(0.0, |screen| screen.frame[1])
}

/// Whether most of `window` is on the displays (`ON_SCREEN`): a window
/// parked off the edge, with only a sliver showing, isn't.
fn on_screen(window: [f64; 4], displays: &[[f64; 4]]) -> bool {
    let area = window[2] * window[3];
    if area <= 0.0 {
        return false;
    }
    let shown: f64 = displays
        .iter()
        .filter_map(|display| screens::intersect(*display, window))
        .map(|[_, _, w, h]| w * h)
        .sum();
    shown / area >= ON_SCREEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parked_window_gets_no_outline_and_one_on_the_screen_does() {
        let display = [[0.0, 0.0, 1800.0, 1169.0]];
        assert!(on_screen([100.0, 80.0, 900.0, 700.0], &display));
        // Parked: hanging off the bottom-right corner but a sliver.
        assert!(!on_screen([1799.0, 1168.0, 900.0, 600.0], &display));
        // Mostly off the right edge.
        assert!(!on_screen([1500.0, 100.0, 900.0, 600.0], &display));
        // Across two displays side by side: whole.
        let two = [[0.0, 0.0, 1800.0, 1169.0], [1800.0, 0.0, 2560.0, 1440.0]];
        assert!(on_screen([1500.0, 100.0, 900.0, 600.0], &two));
    }

    #[test]
    fn an_outlines_label_measures_from_its_own_displays_top() {
        // This Mac: the built-in display, and the M27Q above it.
        let displays = screens::tests::two();
        assert_eq!(display_top(&displays, (100.0, 300.0, 600.0, 400.0)), 0.0);
        assert_eq!(
            display_top(&displays, (100.0, -900.0, 600.0, 400.0)),
            -1440.0
        );
        // Spanning both, its top on the M27Q: the M27Q's top.
        assert_eq!(
            display_top(&displays, (200.0, -100.0, 900.0, 700.0)),
            -1440.0
        );
        // Off every display: the nearest one's.
        assert_eq!(display_top(&displays, (2500.0, 1100.0, 600.0, 400.0)), 0.0);
    }
}
