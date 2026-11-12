//! What the overlays ask of macOS: the displays with their menu bar and
//! Dock (AppKit) and from any thread (CoreGraphics, [`super::displays`]),
//! the pointer, the app and window in front, the panels' place among
//! Spaces and windows, the frame's breathing in Core Animation, and a
//! passive Esc monitor.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSScreen, NSWindow, NSWindowCollectionBehavior,
};
use objc2_foundation::{NSNumber, NSString};

use super::super::frame;
use super::super::screens::Screen;
pub(super) use super::displays::{key, read, take_changed, watch};

/// A task window's frame from `computer::window_frame`, already in global
/// points on macOS.
pub(super) fn from_desktop(frame: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    frame
}

/// Esc's key code.
const ESCAPE: u16 = 53;

/// Every display with its menu bar and Dock, in global points, y down.
pub(super) fn screens() -> Vec<Screen> {
    let Some(mtm) = MainThreadMarker::new() else {
        return read();
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
                key: key(id),
                frame: global(screen.frame()),
                visible: global(screen.visibleFrame()),
                scale: screen.backingScaleFactor(),
            })
        })
        .collect()
}

fn screen_number(screen: &NSScreen) -> Option<u32> {
    let description = screen.deviceDescription();
    let number = description.objectForKey(&NSString::from_str("NSScreenNumber"))?;
    Some(number.downcast_ref::<NSNumber>()?.unsignedIntValue())
}

/// The pointer, in global points.
pub(super) fn pointer() -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let primary = NSScreen::screens(mtm).iter().next()?.frame();
    let top = primary.origin.y + primary.size.height;
    let at = NSEvent::mouseLocation();
    Some((at.x, top - at.y))
}

/// The front window of the app in front, in global points.
pub(super) fn front_window() -> Option<[f64; 4]> {
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

/// The app in front, which a Here question is about.
pub(super) fn frontmost_app() -> Option<String> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(app.localizedName()?.to_string()).filter(|name| !name.is_empty())
}

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
pub(super) fn leave_out_of_mission_control(number: Option<u32>) {
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

/// Orders the outline panel `number` just above window `target` (another
/// app's, by its CoreGraphics number), when it shows. Main thread.
pub(super) fn order_above(number: Option<u32>, target: u32) {
    let Some(panel) = panel(number) else {
        return;
    };
    if !panel.isVisible() {
        return;
    }
    panel.orderWindow_relativeTo(objc2_app_kit::NSWindowOrderingMode::Above, target as isize);
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
pub(super) fn ensure_breathing(number: Option<u32>) {
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

thread_local! {
    /// The global Esc monitor while the bubble or the Hands off pill shows.
    static ESCAPE_MONITOR: std::cell::RefCell<Option<Retained<AnyObject>>> =
        const { std::cell::RefCell::new(None) };
}

/// Watches for Esc in any app (a passive monitor: the key still reaches the
/// app). Esc stops the task moving the real pointer, unless that task
/// pressed it itself, and dismisses the bubble. Main thread only.
pub(super) fn watch_escape(on: bool) {
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
                    super::answer::escape_pressed(pressed_by_person(event));
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
