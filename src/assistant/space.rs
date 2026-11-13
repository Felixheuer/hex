//! Parked windows (`assistant.park_windows`, on by default): the windows
//! Background tasks open kept out of the person's way, with the task stack
//! bringing one forward on request.
//!
//! A real Space of their own isn't possible: macOS has no public API for
//! Spaces, and the private SkyLight calls window managers used
//! (`SLSMoveWindowsToManagedSpace`, creating a Space) work for another
//! app's windows only from inside Dock since macOS 14.5, which yabai reaches
//! by injecting code with System Integrity Protection partly off. Hiding the
//! app hides the person's own windows of it too, and minimizing animates
//! into the Dock.
//!
//! So this does what AeroSpace does for its workspaces: a parked window is
//! moved through Accessibility (Agent-Desk's own `actions::ax`, agent-desktop's
//! `move-window` where that fails) so its
//! top-left corner sits in a bottom corner of a screen, leaving a pixel or
//! two on screen. It stays on the person's Space, in its app, and in
//! Accessibility's tree, so agent-desktop reads and operates it as before;
//! it still shows in Mission Control and the app switcher. [`show_now`]
//! moves it back where it was and brings it to the front. Where it was is
//! kept in the guard's parking store (`guard::parking`), which the guard
//! writes too, so a window goes back even after Agent-Desk quits or crashes. Whether this holds on a
//! given Mac is what `agent-desk dev assistant-space-probe` checks, run only when
//! the person asks.
//!
//! OWNER: browser (with the Space spike).

#[cfg(debug_assertions)]
pub mod probe;

use std::path::Path;

use serde_json::Value;

use crate::assistant::TaskWindow;
use crate::assistant::backends::{self, desktop, search};
use crate::assistant::guard::{self, ParkedHow};

/// Whether moving task windows out of the way is on and can work here
/// (`park_windows` and the assistant are on; on macOS Agent-Desk moves windows
/// itself, elsewhere agent-desktop must be installed).
pub fn available() -> bool {
    let on = crate::app_settings::assistant_runtime()
        .is_some_and(|(lab, settings)| lab && settings.park_windows);
    on && (cfg!(target_os = "macos") || search::find("agent-desktop").is_some())
}

/// A window as it was parked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parked {
    /// Where it was.
    pub from: (f64, f64),
    /// Where it went.
    pub to: (f64, f64),
}

/// Parks `window` off the screen (Hand back, the probe), noting where it was
/// in the guard's parking store first, so it goes back even if Agent-Desk ends.
pub fn park_now(window: &TaskWindow) -> Result<Parked, String> {
    let id = window.window_id.as_deref().ok_or("the window has no id")?;
    let (pid, number, frame) = native::window(id).ok_or("the window isn't listed")?;
    let agents = crate::assistant::computer::displays();
    let setting = crate::assistant::computer::agent_display_setting();
    let preferred = crate::assistant::computer::agents_display(&agents, setting.as_deref())
        .map(|display| display.id.clone());
    let to = parking_spot(&native::displays(), preferred.as_deref()).ok_or("no displays")?;
    guard::note_parked(id, &window.app, Some(frame), ParkedHow::OffScreen);
    native::move_to(pid, number, to)?;
    Ok(Parked {
        from: (frame.0, frame.1),
        to,
    })
}

/// Take over: brings `window` back where it was before Agent-Desk parked it (the
/// parking store's frame, else onto the main display when it hangs off
/// every screen) and to the front; where it is now.
pub fn show_now(window: &TaskWindow) -> Result<(f64, f64), String> {
    put_back(window, true).map(Option::unwrap_or_default)
}

/// A window that left its job: back where it was before Agent-Desk parked it, or
/// onto the main display when it hangs off every screen (parked by an Agent-Desk
/// before the store); whether it moved. It doesn't come to the front.
pub fn bring_back(window: &TaskWindow) -> Result<bool, String> {
    put_back(window, false).map(|to| to.is_some())
}

/// Moves `window` back (`guard::back_to`), fronting its app when `front`;
/// where it is then, or `None` when it was neither parked nor off the
/// screen and isn't fronted.
fn put_back(window: &TaskWindow, front: bool) -> Result<Option<(f64, f64)>, String> {
    let id = window.window_id.as_deref().ok_or("the window has no id")?;
    let (pid, number, frame) = native::window(id).ok_or("the window isn't listed")?;
    let displays = native::displays();
    let record = guard::take_parked(id);
    if record.is_none() && !off_screen(frame, &displays) {
        if front {
            native::activate(pid);
        }
        return Ok(front.then_some((frame.0, frame.1)));
    }
    let from = record.and_then(|record| record.from);
    let to = guard::back_to(from, (frame.2, frame.3), &displays).ok_or("no displays")?;
    native::move_to(pid, number, to)?;
    if front {
        native::activate(pid);
    }
    Ok(Some(to))
}

/// Whether a window of `bounds` (x, y, width, height) hangs off every
/// display but a sliver: less than a tenth of it shows.
pub fn off_screen((x, y, width, height): (f64, f64, f64, f64), displays: &[Display]) -> bool {
    let area = width.max(0.0) * height.max(0.0);
    if area <= 0.0 || displays.is_empty() {
        return false;
    }
    let shown: f64 = displays
        .iter()
        .map(|display| {
            let w = (x + width).min(display.x + display.width) - x.max(display.x);
            let h = (y + height).min(display.y + display.height) - y.max(display.y);
            w.max(0.0) * h.max(0.0)
        })
        .sum();
    shown < area * 0.1
}

/// The parking store's mover in Agent-Desk: the window server for where a window
/// is, Accessibility to move it (`guard::startup`, `guard::put_back_all`).
pub struct Native;

impl guard::ParkingMover for Native {
    fn displays(&mut self) -> Vec<Display> {
        native::displays()
    }

    fn frame(&mut self, id: &str) -> Option<guard::Rect> {
        native::window(id).map(|(_, _, frame)| frame)
    }

    fn move_to(&mut self, window: &guard::ParkedWindow, to: (f64, f64)) -> Result<(), String> {
        let (pid, number, _) = native::window(&window.window_id).ok_or("the window is gone")?;
        native::move_to(pid, number, to)
    }
}

/// Windows moved by Agent-Desk itself: the window server's list for where a
/// window is and whose it is, Accessibility to move it (no driver needed).
mod native {
    use super::Display;

    /// A frame: x, y, width, height in points.
    type Rect = (f64, f64, f64, f64);

    /// Window `id`'s process, number, and frame.
    pub fn window(id: &str) -> Option<(i32, u32, Rect)> {
        if cfg!(test) || !cfg!(target_os = "macos") {
            return None;
        }
        let number = crate::assistant::computer::window_number(id)?;
        crate::assistant::computer::now::all_windows()
            .into_iter()
            .find(|window| window.number == number)
            .map(|window| {
                let [x, y, width, height] = window.bounds;
                (window.pid, number, (x, y, width, height))
            })
    }

    pub fn displays() -> Vec<Display> {
        crate::assistant::computer::displays()
            .iter()
            .map(Display::from)
            .collect()
    }

    /// Moves window `number` of `pid` to `to`, restoring it if minimized.
    /// Never one of Agent-Desk's own windows (its UI tree isn't for worker threads).
    pub fn move_to(pid: i32, number: u32, to: (f64, f64)) -> Result<(), String> {
        if i64::from(pid) == i64::from(std::process::id()) {
            return Err("that's Agent-Desk's own window".into());
        }
        let _ = crate::assistant::actions::ax::set_minimized(pid, number, false);
        crate::assistant::actions::ax::set_position(pid, number, to)
            .map_err(|code| format!("Accessibility couldn't move it (error {code})"))
    }

    #[cfg(target_os = "macos")]
    pub fn activate(pid: i32) {
        use objc2::rc::autoreleasepool;
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        if let Some(app) =
            autoreleasepool(|_| NSRunningApplication::runningApplicationWithProcessIdentifier(pid))
        {
            #[allow(deprecated)]
            let _ = autoreleasepool(|_| {
                app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps)
            });
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn activate(_pid: i32) {}
}

/// One agent-desktop command's data.
pub(crate) fn call(tool: &Path, args: &[&str]) -> Result<Value, String> {
    match backends::desktop_call(tool, args)? {
        desktop::Reply::Ok(data) => Ok(data),
        desktop::Reply::Failed { code, message } => Err(format!("{code}: {message}")),
    }
}

/// `window`'s bounds as `list-windows` gives them; `None` when it isn't
/// listed or has none.
pub(crate) fn bounds(
    tool: &Path,
    app: &str,
    id: &str,
) -> Result<Option<(f64, f64, f64, f64)>, String> {
    let windows = desktop::windows(&call(tool, &["list-windows", "--app", app])?);
    Ok(windows
        .into_iter()
        .find(|window| window.id == id)
        .and_then(|window| window.bounds))
}

/// A display as `list-displays` gives it (top-left origin, points).
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    /// agent-desktop's id ("1", "5").
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub primary: bool,
}

impl From<&crate::assistant::computer::Display> for Display {
    fn from(display: &crate::assistant::computer::Display) -> Self {
        Self {
            id: display.id.clone(),
            x: display.frame.0,
            y: display.frame.1,
            width: display.frame.2,
            height: display.frame.3,
            primary: display.is_primary,
        }
    }
}

impl Display {
    fn contains(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

pub fn displays(data: &Value) -> Vec<Display> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|display| {
            let bounds = display.get("bounds")?;
            let number = |name| bounds.get(name).and_then(Value::as_f64);
            Some(Display {
                id: display
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                x: number("x")?,
                y: number("y")?,
                width: number("width")?,
                height: number("height")?,
                primary: display
                    .get("is_primary")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Where a parked window's top-left corner goes: one point inside the
/// bottom-right corner of a display with no other display to its right or
/// below (else its bottom-left, then the main display's bottom-right), so
/// the window hangs off every screen but a sliver. The agents' display
/// (`preferred`, an id) is tried first, then the main one, so a parked
/// window's sliver stays off the person's working display when it can.
pub fn parking_spot(displays: &[Display], preferred: Option<&str>) -> Option<(f64, f64)> {
    let free = |point: (f64, f64)| !displays.iter().any(|display| display.contains(point));
    let mut ordered: Vec<&Display> = displays.iter().collect();
    ordered.sort_by_key(|display| {
        (
            preferred.is_none_or(|id| display.id != id),
            !display.primary,
        )
    });
    for display in &ordered {
        let (right, bottom) = (display.x + display.width, display.y + display.height);
        if free((right + 1.0, bottom - 1.0))
            && free((right - 1.0, bottom + 1.0))
            && free((right + 1.0, bottom + 1.0))
        {
            return Some((right - 1.0, bottom - 1.0));
        }
    }
    for display in &ordered {
        let (left, bottom) = (display.x, display.y + display.height);
        if free((left - 1.0, bottom - 1.0))
            && free((left + 1.0, bottom + 1.0))
            && free((left - 1.0, bottom + 1.0))
        {
            return Some((left - 1.0, bottom - 1.0));
        }
    }
    let main = ordered.first()?;
    Some((main.x + main.width - 1.0, main.y + main.height - 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: f64, y: f64, width: f64, height: f64, primary: bool) -> Display {
        let id = match (x, y) {
            (0.0, 0.0) => "1",
            (-604.0, -1440.0) => "5",
            _ => "9",
        };
        Display {
            id: id.into(),
            x,
            y,
            width,
            height,
            primary,
        }
    }

    #[test]
    fn a_parked_window_is_off_screen_and_a_shown_one_is_not() {
        let laptop = display(0.0, 0.0, 1800.0, 1169.0, true);
        let screens = [laptop];
        // Parked at the bottom-right corner: one point of it shows.
        assert!(off_screen((1799.0, 1168.0, 900.0, 600.0), &screens));
        // On the screen, or half off it: not parked.
        assert!(!off_screen((80.0, 80.0, 900.0, 600.0), &screens));
        assert!(!off_screen((1350.0, 100.0, 900.0, 600.0), &screens));
        // No displays or no size: nothing to say.
        assert!(!off_screen((1799.0, 1168.0, 900.0, 600.0), &[]));
        assert!(!off_screen((10.0, 10.0, 0.0, 0.0), &screens));
    }

    #[test]
    fn displays_are_read_from_list_displays() {
        let data: Value = serde_json::from_str(
            r#"[{"bounds":{"height":1169.0,"width":1800.0,"x":0.0,"y":0.0},"id":"1","is_primary":true,"scale":2.0},{"bounds":{"height":1440.0,"width":2560.0,"x":-604.0,"y":-1440.0},"id":"5","is_primary":false,"scale":2.0}]"#,
        )
        .unwrap();
        assert_eq!(
            displays(&data),
            [
                display(0.0, 0.0, 1800.0, 1169.0, true),
                display(-604.0, -1440.0, 2560.0, 1440.0, false)
            ]
        );
    }

    #[test]
    fn a_window_parks_in_a_corner_no_other_display_touches() {
        // A laptop with a display above it: its own bottom-right is free.
        let laptop = display(0.0, 0.0, 1800.0, 1169.0, true);
        let above = display(-604.0, -1440.0, 2560.0, 1440.0, false);
        let pair = [laptop.clone(), above.clone()];
        assert_eq!(parking_spot(&pair, None), Some((1799.0, 1168.0)));
        // With the display above as the agents' display, its own free
        // corner: its bottom-right hangs past the laptop's right edge.
        assert_eq!(parking_spot(&pair, Some("5")), Some((1955.0, -1.0)));
        // A taller display to the right of the main one: park on that one.
        let main = display(0.0, 0.0, 1800.0, 1200.0, true);
        let right = display(1800.0, 0.0, 1920.0, 1300.0, false);
        assert_eq!(
            parking_spot(&[main.clone(), right], None),
            Some((3719.0, 1299.0))
        );
        // A shorter one: the main display's corner hangs below it.
        let short = display(1800.0, 0.0, 1920.0, 1080.0, false);
        assert_eq!(parking_spot(&[main, short], None), Some((1799.0, 1199.0)));
        assert_eq!(parking_spot(&[], None), None);
    }

    #[test]
    fn three_displays_left_right_and_above_park_off_a_free_corner() {
        // Main 1440×900 at 0,0; one left (1920×1080 at -1920,0), one right
        // (1280×1024 at 1440,0), one above (2560×1440 at -560,-1440).
        let main = display(0.0, 0.0, 1440.0, 900.0, true);
        let mut left = display(-1920.0, 0.0, 1920.0, 1080.0, false);
        left.id = "2".into();
        let mut right = display(1440.0, 0.0, 1280.0, 1024.0, false);
        right.id = "3".into();
        let mut above = display(-560.0, -1440.0, 2560.0, 1440.0, false);
        above.id = "4".into();
        let all = [main, left, right, above];
        // The main display's bottom-right touches the right display, so the
        // next display with a free corner (the left one, listed first).
        let spot = parking_spot(&all, None).unwrap();
        assert_eq!(spot, (-1.0, 1079.0));
        assert!(
            all.iter()
                .all(|display| { !display.contains((spot.0 + 2.0, spot.1 + 2.0)) })
        );
        // The agents' display (right) parks at its own corner.
        assert_eq!(parking_spot(&all, Some("3")), Some((2719.0, 1023.0)));
        // The display above has no free corner of its own (main and the
        // side displays touch its bottom edge): the next free one.
        assert_eq!(parking_spot(&all, Some("4")), Some((-1.0, 1079.0)));
    }
}
