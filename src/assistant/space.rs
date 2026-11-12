//! Experimental (`assistant.agent_space`): the windows Background tasks open
//! kept out of the person's way, with the task stack bringing one forward
//! on request.
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
//! moved through Accessibility (agent-desktop's `move-window`) so its
//! top-left corner sits in a bottom corner of a screen, leaving a pixel or
//! two on screen. It stays on the person's Space, in its app, and in
//! Accessibility's tree, so agent-desktop reads and operates it as before;
//! it still shows in Mission Control and the app switcher. [`show`] moves
//! it back where it was and brings it to the front. Whether this holds on a
//! given Mac is what `hex dev assistant-space-probe` checks, run only when
//! the person asks.
//!
//! OWNER: browser (with the Space spike).

#[cfg(debug_assertions)]
pub mod probe;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::assistant::TaskWindow;
use crate::assistant::backends::{self, desktop, search};

/// Where parked windows were, by window id, to put them back.
static PARKED: Mutex<Option<HashMap<String, (f64, f64)>>> = Mutex::new(None);

/// Whether moving task windows out of the way is on and can work here
/// (`agent_space` and the assistant are on, and agent-desktop is installed).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the agent_space setting is not wired to tasks yet"
    )
)]
pub fn available() -> bool {
    let on = crate::app_settings::assistant_runtime()
        .is_some_and(|(lab, settings)| lab && settings.agent_space);
    on && search::find("agent-desktop").is_some()
}

/// Moves `window` out of the person's way; false when it couldn't (or
/// `agent_space` is off).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the agent_space setting is not wired to tasks yet"
    )
)]
pub fn park(window: &TaskWindow) -> bool {
    available()
        && park_now(window)
            .inspect_err(|error| tracing::warn!(error, app = window.app, "couldn't park a window"))
            .is_ok()
}

/// Brings `window` back where it was and to the front; false when it
/// couldn't (or `agent_space` is off).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the agent_space setting is not wired to tasks yet"
    )
)]
pub fn show(window: &TaskWindow) -> bool {
    available()
        && show_now(window)
            .inspect_err(|error| tracing::warn!(error, app = window.app, "couldn't show a window"))
            .is_ok()
}

/// A window as it was parked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parked {
    /// Where it was.
    pub from: (f64, f64),
    /// Where it went.
    pub to: (f64, f64),
}

/// Parks `window` whatever the setting says (the probe's way in).
pub fn park_now(window: &TaskWindow) -> Result<Parked, String> {
    let tool = agent_desktop()?;
    let id = window
        .window_id
        .as_deref()
        .ok_or("the window has no agent-desktop id")?;
    let (x, y, _, _) = bounds(&tool, &window.app, id)?
        .ok_or("agent-desktop doesn't list the window's position")?;
    let displays = displays(&call(&tool, &["list-displays"])?);
    let agents = crate::assistant::computer::displays();
    let setting = crate::assistant::computer::agent_display_setting();
    let preferred = crate::assistant::computer::agents_display(&agents, setting.as_deref())
        .map(|display| display.id.clone());
    let to =
        parking_spot(&displays, preferred.as_deref()).ok_or("agent-desktop listed no displays")?;
    move_window(&tool, &window.app, id, to)?;
    PARKED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(id.to_owned(), (x, y));
    Ok(Parked { from: (x, y), to })
}

/// Shows `window` whatever the setting says; where it was parked from.
pub fn show_now(window: &TaskWindow) -> Result<(f64, f64), String> {
    let tool = agent_desktop()?;
    let id = window
        .window_id
        .as_deref()
        .ok_or("the window has no agent-desktop id")?;
    let remembered = PARKED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
        .and_then(|parked| parked.remove(id));
    let to = match remembered {
        Some(spot) => spot,
        // Parked by another process: somewhere on the main display.
        None => {
            let displays = displays(&call(&tool, &["list-displays"])?);
            let main = displays
                .iter()
                .find(|display| display.primary)
                .or(displays.first())
                .ok_or("agent-desktop listed no displays")?;
            (main.x + 80.0, main.y + 80.0)
        }
    };
    move_window(&tool, &window.app, id, to)?;
    if let Err(error) = call(
        &tool,
        &["focus-window", "--app", &window.app, "--window-id", id],
    ) {
        tracing::debug!(error, "the shown window didn't come to the front");
    }
    Ok(to)
}

fn agent_desktop() -> Result<PathBuf, String> {
    search::find("agent-desktop").ok_or_else(|| "agent-desktop isn't installed".into())
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

fn move_window(tool: &Path, app: &str, id: &str, (x, y): (f64, f64)) -> Result<(), String> {
    let x = format!("--x={}", x.round() as i64);
    let y = format!("--y={}", y.round() as i64);
    call(
        tool,
        &["move-window", "--app", app, "--window-id", id, &x, &y],
    )
    .map(|_| ())
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

    #[test]
    fn nothing_moves_while_the_experiment_is_off() {
        // Without applied settings (tests), the setting reads as off.
        if crate::app_settings::assistant_runtime().is_some() {
            return;
        }
        let window = TaskWindow {
            app: "TextEdit".into(),
            window_id: Some("w-1".into()),
        };
        assert!(!available());
        assert!(!park(&window));
        assert!(!show(&window));
    }
}
