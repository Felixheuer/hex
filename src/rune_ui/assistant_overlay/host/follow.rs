//! Which display the HUD-side overlays show on (the orb, the Here bubble,
//! the hint, the Hands off pill): the one the person works on, its front
//! window's, else the pointer's. They're placed there, and move there when
//! the person switches displays or displays come and go (the watcher asks
//! once a second while there's work, and at once when displays change).
//! Main thread.

use std::sync::Mutex;
use std::time::Duration;

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSEvent, NSScreen};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSNumber, NSString};
use rune::{DisplayTarget, OverlayHandle, OverlayPlacement, ScreenEdge};

use super::super::orb::Orb;
use super::super::place;
use super::super::screens::{self, Screen};
use super::{OVERLAYS, displays};
use crate::rune_ui::live_text::Spot;

/// How a panel that moved to another display comes in there.
const MOVE_IN: Duration = Duration::from_millis(240);

/// Every display with its taskbar or panels, in global points, y down.
#[cfg(not(target_os = "macos"))]
pub(super) fn screens() -> Vec<Screen> {
    displays::read()
}

/// The pointer, in global points.
#[cfg(not(target_os = "macos"))]
pub(super) fn pointer() -> Option<(f64, f64)> {
    displays::pointer()
}

/// The front window, in global points.
#[cfg(not(target_os = "macos"))]
fn front_window() -> Option<[f64; 4]> {
    displays::front_window()
}

/// Every display with its menu bar and Dock, in global points, y down.
#[cfg(target_os = "macos")]
pub(super) fn screens() -> Vec<Screen> {
    let Some(mtm) = MainThreadMarker::new() else {
        return displays::read();
    };
    let all = NSScreen::screens(mtm);
    let Some(primary) = all.iter().next() else {
        return Vec::new();
    };
    let top = primary.frame().origin.y + primary.frame().size.height;
    let global = |r: objc2_foundation::NSRect| {
        [
            r.origin.x,
            top - (r.origin.y + r.size.height),
            r.size.width,
            r.size.height,
        ]
    };
    all.iter()
        .filter_map(|screen| {
            let id = screen_number(&screen)?;
            Some(Screen {
                id,
                key: displays::key(id),
                frame: global(screen.frame()),
                visible: global(screen.visibleFrame()),
                scale: screen.backingScaleFactor(),
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn screen_number(screen: &NSScreen) -> Option<u32> {
    let description = screen.deviceDescription();
    let number = description.objectForKey(&NSString::from_str("NSScreenNumber"))?;
    Some(number.downcast_ref::<NSNumber>()?.unsignedIntValue())
}

/// The pointer, in global points.
#[cfg(target_os = "macos")]
pub(super) fn pointer() -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let primary = NSScreen::screens(mtm).iter().next()?.frame();
    let top = primary.origin.y + primary.size.height;
    let at = NSEvent::mouseLocation();
    Some((at.x, top - at.y))
}

/// The front window of the app in front, in global points.
#[cfg(target_os = "macos")]
fn front_window() -> Option<[f64; 4]> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app.processIdentifier();
    let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
    windows
        .iter()
        .find(|window| {
            window.pid == pid
                && window.layer == 0
                && window.alpha > 0.0
                && window.bounds[2] > 1.0
                && window.bounds[3] > 1.0
        })
        .map(|window| window.bounds)
}

/// The display the person works on, and every display.
pub(super) fn working() -> Option<(Screen, Vec<Screen>)> {
    let all = screens();
    let screen = screens::working(&all, front_window(), pointer()?)?.clone();
    Some((screen, all))
}

/// The spot past the HUD on `screen` (global points, and whether it hangs
/// below a HUD at the top): where the HUD says when it's on that display,
/// else the same place carried over from the HUD's.
pub(super) fn hud_spot(screen: &Screen, all: &[Screen]) -> (f64, f64, bool) {
    match crate::rune_hud::live_text_place() {
        Spot::Global { x, y, below } => {
            let at = (f64::from(x), f64::from(y));
            match screens::at(all, at) {
                Some(from) if from.id != screen.id => {
                    let (x, y) = screens::carry(at, below, from, screen);
                    (x, y, below)
                }
                _ => (at.0, at.1, below),
            }
        }
        Spot::Edge(edge, margin) => {
            let [x, y, w, h] = screen.frame;
            let below = edge == ScreenEdge::Top;
            let margin = f64::from(margin);
            let spot_y = if below { y + margin } else { y + h - margin };
            (x + w / 2.0, spot_y, below)
        }
    }
}

/// A panel's rect, in global points.
pub(super) fn set_rect<T>(handle: &OverlayHandle<T>, [x, y, width, height]: [f32; 4]) {
    handle.set_display(DisplayTarget::Primary);
    handle.set_placement(Some(OverlayPlacement::Rect {
        x,
        y,
        width,
        height,
    }));
}

/// Places a pill of `size` where Live text would start on the working
/// display, past its plate when Live text may show one there.
pub(super) fn place_past_hud<T>(handle: &OverlayHandle<T>, (width, height): (f32, f32)) {
    let Some((screen, all)) = working() else {
        return;
    };
    let past = if crate::labs::live_text() {
        f64::from(crate::rune_ui::live_text::SIZE.1)
    } else {
        0.0
    };
    let (x, y, below) = hud_spot(&screen, &all);
    let (w, h) = (f64::from(width), f64::from(height));
    let top = if below { y + past } else { y - h - past };
    let rect = screens::snap([x - w / 2.0, top, w, h], screen.scale);
    set_rect(handle, rect.map(|v| v as f32));
    note_placed(screen.id);
}

/// Where the orb's centre is now, in global points.
static ORB_AT: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// The display the HUD-side panels were last placed on.
static PLACED_ON: Mutex<Option<u32>> = Mutex::new(None);

fn note_placed(id: u32) {
    *PLACED_ON.lock().unwrap_or_else(|error| error.into_inner()) = Some(id);
}

/// Puts the orb on the working display: where it was dragged there, else
/// beside the HUD.
pub(super) fn place_orb(handle: &OverlayHandle<Orb>, orb: &mut Orb) {
    let Some((screen, all)) = working() else {
        return;
    };
    let (x, _, below) = hud_spot(&screen, &all);
    let anchor = place::anchor(&screen.key, screen.visible, Some((x, below)));
    orb.grow = place::grow(anchor, screen.visible);
    *ORB_AT.lock().unwrap_or_else(|error| error.into_inner()) = Some(anchor);
    let rect = screens::snap(place::rect(anchor, orb.grow).map(f64::from), screen.scale);
    set_rect(handle, rect.map(|v| v as f32));
    note_placed(screen.id);
}

/// A point in global points.
type Point = (f64, f64);

thread_local! {
    /// While the orb is dragged: the pointer and the orb's centre when it
    /// was pressed.
    static DRAG: std::cell::Cell<Option<(Point, Point)>> =
        const { std::cell::Cell::new(None) };
}

/// Drags the orb with the pointer, across displays too; on release, keeps
/// it inside the display it was dropped on, turns what unfolds from it
/// toward that display, and remembers the spot for that display.
pub(super) fn drag_orb(orb: &mut Orb, drag: &rune::Drag) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let Some(pointer) = pointer() else {
        return;
    };
    let at = *ORB_AT.lock().unwrap_or_else(|error| error.into_inner());
    match drag.phase {
        rune::Phase::Down => DRAG.set(at.map(|anchor| (pointer, anchor))),
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
            let all = screens();
            let Some(screen) = screens::at(&all, pointer) else {
                return;
            };
            let dropped = if drag.cancelled() {
                anchor
            } else {
                place::clamp(
                    (anchor.0 + pointer.0 - from.0, anchor.1 + pointer.1 - from.1),
                    screen.visible,
                )
            };
            orb.grow = place::grow(dropped, screen.visible);
            let rect = screens::snap(place::rect(dropped, orb.grow).map(f64::from), screen.scale);
            set_rect(&overlays.orb, rect.map(|v| v as f32));
            if !drag.cancelled() {
                place::remember(&screen.key, screen.visible, dropped);
            }
            *ORB_AT.lock().unwrap_or_else(|error| error.into_inner()) = Some(dropped);
            note_placed(screen.id);
        }
    }
}

/// Moves the HUD-side panels that show to the working display when the
/// person switched displays, or (`force`) displays came, went, or moved.
/// Any thread.
pub(super) fn follow(force: bool) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    overlays.orb.update_if(move |orb, _| {
        follow_now(orb, force);
        false
    });
}

fn follow_now(orb: &mut Orb, force: bool) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let Some((screen, _)) = working() else {
        return;
    };
    let before = *PLACED_ON.lock().unwrap_or_else(|error| error.into_inner());
    if !force && before.is_none_or(|id| id == screen.id) {
        return;
    }
    if orb.shown {
        overlays.orb.hide();
        place_orb(&overlays.orb, orb);
        overlays.orb.fade_in(MOVE_IN);
    }
    overlays.bubble.update_if(|state, _| {
        if let Some(overlays) = OVERLAYS.get()
            && state.shown
        {
            overlays.bubble.hide();
            state.below = super::place_bubble(&overlays.bubble);
            overlays.bubble.fade_in(MOVE_IN);
        }
        false
    });
    if super::hands_task().is_some() {
        overlays.hands.hide();
        place_past_hud(&overlays.hands, super::super::hands::SIZE);
        overlays.hands.fade_in(MOVE_IN);
    }
    overlays.listening.update_if(|state, _| {
        if let Some(overlays) = OVERLAYS.get()
            && state.since.is_some()
        {
            place_past_hud(&overlays.listening, super::super::listening::SIZE);
        }
        false
    });
    if super::hint_shown() {
        place_past_hud(&overlays.hint, super::super::hint::SIZE);
    }
    note_placed(screen.id);
}
