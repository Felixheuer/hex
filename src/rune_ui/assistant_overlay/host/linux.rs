//! What the overlays ask of Linux: nothing at global coordinates. Wayland
//! has none (a layer-shell surface sits on an output, anchored to its
//! edges), and one way for X11 and Wayland keeps the overlays alike on
//! every desktop. So no display, pointer, or window is read here: the orb,
//! the bubble, and the pills sit at the HUD's edge of the HUD's display
//! (`follow`), the frame covers that display (`watcher`), and no task
//! window is outlined (its frame couldn't be placed). There are no Spaces,
//! Core Animation, or passive Esc monitor either: the frame stands still,
//! and the listener takes Esc while a Here task runs
//! (`assistant::trigger::claims_escape`), on X11 only, as the shortcuts
//! portal can't hold a bare Esc.

use super::super::screens::Screen;

/// No display is read: the overlays place themselves by the HUD instead.
pub(super) fn screens() -> Vec<Screen> {
    Vec::new()
}

pub(super) fn read() -> Vec<Screen> {
    Vec::new()
}

/// No pointer is read: the orb isn't dragged on Linux.
pub(super) fn pointer() -> Option<(f64, f64)> {
    None
}

pub(super) fn front_window() -> Option<[f64; 4]> {
    None
}

/// A task window's frame, unused: nothing is outlined on Linux.
pub(super) fn from_desktop(frame: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    frame
}

/// The Here hint names no app on Linux: the focused window can take an
/// AT-SPI walk to name, which mustn't hold up the HUD's thread.
pub(super) fn frontmost_app() -> Option<String> {
    None
}

pub(super) fn take_changed() -> bool {
    false
}

pub(super) fn watch(_: fn()) {}

pub(super) fn leave_out_of_mission_control(_: Option<u32>) {}

pub(super) fn order_above(_: Option<u32>, _: u32) {}

pub(super) fn ensure_breathing(_: Option<u32>) {}

pub(super) fn watch_escape(_: bool) {}
