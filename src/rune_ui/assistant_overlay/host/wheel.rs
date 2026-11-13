//! The wheel and the trackpad on the panels (`super::super::wheel`): Rune's
//! overlay host drops scroll events, so an AppKit local monitor hears each
//! one sent to a panel (key or not: AppKit sends a scroll to the window
//! under the pointer, and a panel takes it wherever it takes clicks),
//! finds which panel and where, and moves the viewport there. The event
//! goes on to the panel unchanged. Main thread.

use super::super::wheel::{self, Scroll, Surface};
use super::OVERLAYS;

/// The panel with window `number`.
fn surface_of(number: u32) -> Option<Surface> {
    let overlays = OVERLAYS.get()?;
    let is = |at: Option<u32>| at == Some(number);
    if is(overlays.orb.window_number()) {
        Some(Surface::Orb)
    } else if is(overlays.wall.window_number()) {
        Some(Surface::Wall)
    } else if is(overlays.stage.window_number()) {
        Some(Surface::Stage)
    } else if is(overlays.enlarged.window_number()) {
        Some(Surface::Enlarged)
    } else {
        None
    }
}

/// Moves a viewport in `surface`.
fn apply(surface: Surface, scroll: Scroll) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    match surface {
        Surface::Orb => overlays.orb.update(move |_, tx| scroll.apply(tx)),
        Surface::Wall => overlays.wall.update(move |_, tx| scroll.apply(tx)),
        Surface::Stage => overlays.stage.update(move |_, tx| scroll.apply(tx)),
        Surface::Enlarged => overlays.enlarged.update(move |_, tx| scroll.apply(tx)),
    }
}

/// A delta of the wheel (in lines) or the trackpad (in points), in points
/// moving content with the fingers, as Rune's windows read it.
pub(in super::super) fn points(precise: bool, (dx, dy): (f64, f64)) -> (f32, f32) {
    let per = if precise { 1.0 } else { 40.0 };
    ((dx * per) as f32, (dy * per) as f32)
}

/// A scroll at `point` (panel points, y down) in window `number`.
pub(in super::super) fn heard(number: u32, point: (f32, f32), delta: (f32, f32)) {
    let Some(surface) = surface_of(number) else {
        return;
    };
    if let Some(scroll) = wheel::route(surface, point, delta) {
        apply(surface, scroll);
    }
}

/// Starts listening (once).
#[cfg(target_os = "macos")]
pub(super) fn install() {
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    let block = block2::RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a live event for the handler's duration.
        let at = unsafe { event.as_ref() };
        if let Some(mtm) = objc2::MainThreadMarker::new()
            && let Some(window) = at.window(mtm)
        {
            let number = at.windowNumber().max(0) as u32;
            let point = at.locationInWindow();
            let height = window.frame().size.height;
            let delta = points(
                at.hasPreciseScrollingDeltas(),
                (at.scrollingDeltaX(), at.scrollingDeltaY()),
            );
            heard(number, (point.x as f32, (height - point.y) as f32), delta);
        }
        event.as_ptr()
    });
    // SAFETY: the block lives as long as AppKit retains it; the monitor
    // stays for the app's life.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, &block)
    };
    // It stays for the app's life.
    std::mem::forget(monitor);
}

/// Elsewhere Rune's overlays are layer surfaces or X11 windows whose wheel
/// isn't routed yet.
#[cfg(not(target_os = "macos"))]
pub(super) fn install() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_notch_is_forty_points_and_a_trackpad_moves_one_to_one() {
        assert_eq!(points(false, (0.0, -1.0)), (0.0, -40.0));
        assert_eq!(points(true, (-3.5, 12.0)), (-3.5, 12.0));
    }
}
