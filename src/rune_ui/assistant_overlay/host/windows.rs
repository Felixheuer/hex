//! What the overlays ask of Windows: the displays with their taskbars, the
//! pointer, and the app and window in front, in Rune's global points (the
//! primary display's top-left as origin, desktop pixels divided by the
//! primary display's scale, which is exact where displays share a scale).
//! Windows has no Spaces, no Core Animation, and no passive Esc monitor:
//! panels need no Space behaviour, the frame stands still rather than
//! breathing, and the listener takes Esc while a Here task runs
//! (`assistant::trigger::claims_escape`).

use std::sync::Mutex;

use windows_sys::Win32::Foundation::{LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowRect, MONITORINFOF_PRIMARY,
};

use super::super::screens::Screen;

/// One monitor as Windows reports it: desktop pixels and its scale.
struct Monitor {
    handle: HMONITOR,
    frame: RECT,
    work: RECT,
    scale: f64,
    primary: bool,
}

fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        found: LPARAM,
    ) -> windows_sys::core::BOOL {
        // SAFETY: `found` is the Vec passed below, alive for the call.
        let found = unsafe { &mut *(found as *mut Vec<Monitor>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: a monitor Windows just listed, and a sized struct.
        if unsafe { GetMonitorInfoW(handle, &mut info) } != 0 {
            let (mut dpi_x, mut dpi_y) = (96, 96);
            // SAFETY: as above; a failure leaves 96 DPI (1×).
            unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            found.push(Monitor {
                handle,
                frame: info.rcMonitor,
                work: info.rcWork,
                scale: f64::from(dpi_x.max(1)) / 96.0,
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        1
    }
    let mut found: Vec<Monitor> = Vec::new();
    // SAFETY: the callback only touches `found`, which outlives the call.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut found as *mut Vec<Monitor> as LPARAM,
        );
    }
    // The primary display first, as on macOS.
    found.sort_by_key(|monitor| !monitor.primary);
    found
}

/// Pixels per point on the primary display, Rune's unit for global rects.
fn primary_scale(monitors: &[Monitor]) -> f64 {
    monitors
        .iter()
        .find(|monitor| monitor.primary)
        .map_or(1.0, |monitor| monitor.scale)
}

fn rect(r: RECT, scale: f64) -> [f64; 4] {
    [
        f64::from(r.left) / scale,
        f64::from(r.top) / scale,
        f64::from(r.right - r.left) / scale,
        f64::from(r.bottom - r.top) / scale,
    ]
}

/// Every display with its taskbar, in global points, the primary first.
pub(super) fn screens() -> Vec<Screen> {
    let all = monitors();
    let scale = primary_scale(&all);
    all.iter()
        .map(|monitor| Screen {
            id: monitor.handle as usize as u32,
            key: format!(
                "{}x{}@{},{}",
                monitor.frame.right - monitor.frame.left,
                monitor.frame.bottom - monitor.frame.top,
                monitor.frame.left,
                monitor.frame.top
            ),
            frame: rect(monitor.frame, scale),
            visible: rect(monitor.work, scale),
            scale: monitor.scale,
        })
        .collect()
}

/// The displays from any thread: the same read, which Windows allows off
/// the UI thread.
pub(super) fn read() -> Vec<Screen> {
    screens()
}

/// The pointer, in global points.
pub(super) fn pointer() -> Option<(f64, f64)> {
    let mut at = POINT { x: 0, y: 0 };
    // SAFETY: writes the cursor position into `at`.
    if unsafe { GetCursorPos(&mut at) } == 0 {
        return None;
    }
    let scale = primary_scale(&monitors());
    Some((f64::from(at.x) / scale, f64::from(at.y) / scale))
}

/// The window in front, in global points.
pub(super) fn front_window() -> Option<[f64; 4]> {
    // SAFETY: plain reads of the foreground window and its rect.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return None;
    }
    let mut frame = RECT::default();
    if unsafe { GetWindowRect(window, &mut frame) } == 0 {
        return None;
    }
    Some(rect(frame, primary_scale(&monitors())))
}

/// A task window's frame from `computer::window_frame`, which reads desktop
/// pixels on Windows, in the overlays' global points.
pub(super) fn from_desktop((x, y, w, h): (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let scale = primary_scale(&monitors());
    (x / scale, y / scale, w / scale, h / scale)
}

/// The app in front, which a Here question is about: its executable's name.
pub(super) fn frontmost_app() -> Option<String> {
    let stem = crate::windows::input::foreground_process_stem()?;
    let mut letters = stem.chars();
    let first = letters.next()?;
    Some(first.to_uppercase().chain(letters).collect())
}

/// What the displays were at the last look, so a change shows.
static SEEN: Mutex<Vec<[f64; 4]>> = Mutex::new(Vec::new());

/// Whether the displays changed since the last look (Windows calls no one
/// back; the watcher looks each second while anything shows).
pub(super) fn take_changed() -> bool {
    let now: Vec<_> = screens().into_iter().map(|screen| screen.frame).collect();
    let mut seen = SEEN.lock().unwrap_or_else(|error| error.into_inner());
    let changed = !seen.is_empty() && *seen != now;
    *seen = now;
    changed
}

/// Displays coming and going are found by [`take_changed`]'s look instead.
pub(super) fn watch(_: fn()) {}

/// Windows has no Spaces or Mission Control to keep a panel out of.
pub(super) fn leave_out_of_mission_control(_: Option<u32>) {}

/// An outline stays over every window, as the other overlays do.
pub(super) fn order_above(_: Option<u32>, _: u32) {}

/// The frame stands still: breathing would redraw it all the time.
pub(super) fn ensure_breathing(_: Option<u32>) {}

/// The listener takes Esc while a Here task runs or speaks; the bubble and
/// the Hands off pill need no monitor of their own.
pub(super) fn watch_escape(_: bool) {}
