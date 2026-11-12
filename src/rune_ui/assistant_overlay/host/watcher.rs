//! The watcher thread: shows the screen frame on every display while
//! Background work runs, waits, or is queued, moves each outline with its
//! window, and, while a task might hold the real pointer, asks the guard
//! who does. It sleeps while there's nothing to show, move, or ask.

use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use objc2::msg_send;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSNumber, NSString};
use rune::{DisplayTarget, OverlayPlacement};

use super::super::board::Outlined;
#[cfg(target_os = "macos")]
use super::super::frame;
use super::super::frame::Frame;
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

/// What the watcher thread watches, set from the subscriber and the orb.
#[derive(Default)]
struct Watch {
    frame: bool,
    outlined: Vec<Outlined>,
    /// A task is live, so one might take the real pointer.
    live: bool,
    /// Bumped on every change, so the watcher wakes for it.
    generation: u64,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch {
    frame: false,
    outlined: Vec::new(),
    live: false,
    generation: 0,
});
static WATCH_CHANGED: Condvar = Condvar::new();
static WATCHER: OnceLock<()> = OnceLock::new();

fn watch() -> MutexGuard<'static, Watch> {
    WATCH.lock().unwrap_or_else(|error| error.into_inner())
}

/// Shows or hides the screen frame.
pub(super) fn set_frame(on: bool) {
    change(|watch| std::mem::replace(&mut watch.frame, on) != on);
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

/// Starts the watcher on first use, so a Hex that never uses the assistant
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

/// What one frame panel shows: its display and how it meets the others.
#[derive(Clone, Debug, PartialEq)]
struct FrameSlot {
    screen: Screen,
    seams: Seams,
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
}

impl Watcher {
    fn run(mut self) {
        self.frames = vec![None; MAX_DISPLAYS];
        self.slots = vec![None; MAX_OUTLINES];
        let mut seen = u64::MAX;
        loop {
            let (frame, outlined, live) = {
                let mut watch = watch();
                // Sleep while there's nothing to show, move, or ask.
                while !watch.frame
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
                (watch.frame, watch.outlined.clone(), watch.live)
            };
            if displays::take_changed() {
                // Read them again now, not within the second.
                self.displays_read = None;
            }
            self.frames(frame);
            self.outlines(&outlined);
            self.ask_guard(live);
            if (frame || live) && self.followed.is_none_or(|at| at.elapsed() >= DISPLAY_POLL) {
                self.followed = Some(Instant::now());
                // The orb and bubble move to the display the person works on.
                super::follow::follow(false);
            }
            let pause = if !outlined.is_empty() {
                OUTLINE_POLL
            } else if frame || live {
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

    /// Shows a frame on every display while Background work runs, each
    /// joined to its neighbours, follows displays coming, going, moving,
    /// and resizing (a panel stays with its display), and keeps each
    /// breathing.
    fn frames(&mut self, on: bool) {
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        let read = on && self.read_displays();
        let wanted: Vec<FrameSlot> = if on {
            self.displays
                .iter()
                .map(|screen| FrameSlot {
                    screen: screen.clone(),
                    seams: screens::seams(screen, &self.displays),
                })
                .collect()
        } else {
            Vec::new()
        };
        // Displays that went: their panels go.
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
                };
                handle.update(move |frame, _| *frame = state);
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
            let moved = have.as_ref() != Some(&want);
            self.frames[index] = Some(want);
            // Checked with each read of the displays, not each outline poll.
            if read || moved {
                let breathing = handle.clone();
                // On the main thread; no redraw.
                handle.update_if(move |_, _| {
                    ensure_breathing(breathing.window_number());
                    false
                });
            }
        }
    }

    /// Moves each live task window's outline to where its window is now: in
    /// one piece per display it spans, and none for a window on no display
    /// (one left where an unplugged display was).
    fn outlines(&mut self, outlined: &[Outlined]) {
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        if !outlined.is_empty() {
            self.read_displays();
        }
        // Each outline's pieces now.
        let mut wanted: Vec<Piece<'_>> = Vec::new();
        for item in outlined {
            let Some(window) = super::window_frame(&item.window, &self.displays)
                .filter(|(_, _, w, h)| *w >= 1.0 && *h >= 1.0)
            else {
                continue;
            };
            let window_rect = [window.0, window.1, window.2, window.3];
            if !self
                .displays
                .iter()
                .any(|screen| screens::intersect(screen.frame, window_rect).is_some())
            {
                continue;
            }
            let top = display_top(&self.displays, window);
            let (rect, label) = outline::rect(window, top);
            let full = rect.map(f64::from);
            for (display, piece) in screens::pieces(full, &self.displays) {
                wanted.push((item, display, full, piece, label, (window.2, window.3)));
            }
        }
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

/// Whether macOS asks for less motion (Accessibility › Display › Reduce
/// motion).
#[cfg(target_os = "macos")]
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(target_os = "macos")]
const BREATHE_KEY: &str = "hex-breathe";

/// Linux and Windows: the frame stays still (no window server animates it).
#[cfg(not(target_os = "macos"))]
fn ensure_breathing(_number: Option<u32>) {}

/// Breathes the frame panel's layer in Core Animation (none with Reduce
/// motion on): the window server animates it, so Hex draws nothing for it.
/// Added again if the layer was remade.
#[cfg(target_os = "macos")]
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
