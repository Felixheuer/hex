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
//! it still shows in Mission Control and the app switcher. [`show_now`] moves
//! it back where it was and brings it to the front. Whether this holds on a
//! given Mac is what `hex dev assistant-space-probe` checks, run only when
//! the person asks.
//!
//! The same works on Windows (UI Automation reads a window wherever it is;
//! [`sliver`] keeps part of it past the invisible resize border) and on
//! Linux under X11. Wayland lets no app move another's windows, so there
//! [`available`] says no and the switch says why ([`unsupported_here`]).
//! Coordinates are agent-desktop's own (`list-displays` and `move-window`
//! share them on every OS).
//!
//! The service parks a Background task's windows once it opened or claimed
//! them ([`park_held`], from the guard's leases), and brings them back when
//! the person takes the task over or brings it forward from the task stack,
//! and when it ends ([`show_task`]).
//!
//! OWNER: browser (with the Space spike).

#[cfg(all(debug_assertions, target_os = "macos"))]
pub mod probe;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::assistant::TaskWindow;
use crate::assistant::backends::os::Os;
use crate::assistant::backends::{self, desktop, search};

/// Where parked windows were, by window id, to put them back.
static PARKED: Mutex<Option<HashMap<String, (f64, f64)>>> = Mutex::new(None);

/// Each task's parked windows, and those the person brought forward (not
/// parked again).
static TASKS: Mutex<Option<HashMap<String, TaskSpace>>> = Mutex::new(None);

#[derive(Default)]
struct TaskSpace {
    parked: Vec<TaskWindow>,
    kept: HashSet<String>,
    /// A look at its leases is under way.
    parking: bool,
}

fn tasks() -> std::sync::MutexGuard<'static, Option<HashMap<String, TaskSpace>>> {
    TASKS.lock().unwrap_or_else(|error| error.into_inner())
}

/// Whether agent-desktop can move other apps' windows on `os`: everywhere
/// but a Wayland session, where only the compositor places windows.
pub fn moves_windows(os: Os, wayland: bool) -> bool {
    !(os == Os::Linux && wayland)
}

/// Why task windows can't be moved out of the way here, if they can't.
pub fn unsupported_here() -> Option<&'static str> {
    (!moves_windows(Os::HERE, wayland())).then_some(
        "Wayland doesn't let one app move another's windows, so they stay where they open",
    )
}

fn wayland() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::linux::session::LinuxSession::detect().is_wayland()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Whether moving task windows out of the way is on and can work here
/// (`agent_space` and the assistant are on, agent-desktop is installed,
/// and this session lets it move windows).
pub fn available() -> bool {
    let on = crate::app_settings::assistant_runtime()
        .is_some_and(|(lab, settings)| lab && settings.agent_space);
    on && unsupported_here().is_none() && search::find("agent-desktop").is_some()
}

/// Moves `window` out of the person's way; false when it couldn't (or
/// `agent_space` is off).
pub fn park(window: &TaskWindow) -> bool {
    available()
        && park_now(window)
            .inspect_err(|error| tracing::warn!(error, app = window.app, "couldn't park a window"))
            .is_ok()
}

/// Parks the windows Background task `task` holds (it opened or claimed
/// them; the guard's leases say which) that aren't parked yet and that the
/// person didn't bring forward. Returns at once; the moves happen on a
/// thread of their own, one look per task at a time.
pub fn park_held(task: &str) {
    if !available() {
        return;
    }
    {
        let mut tasks = tasks();
        let space = tasks
            .get_or_insert_with(HashMap::new)
            .entry(task.to_owned())
            .or_default();
        if space.parking {
            return;
        }
        space.parking = true;
    }
    let task = task.to_owned();
    let spawned = std::thread::Builder::new()
        .name("hex-assistant-space".into())
        .spawn({
            let task = task.clone();
            move || park_leases(&task)
        });
    if spawned.is_err() {
        finish_parking(&task, Vec::new());
    }
}

fn park_leases(task: &str) {
    let held: Vec<TaskWindow> = crate::assistant::guard::leases()
        .into_iter()
        .filter(|lease| lease.task == task)
        .map(|lease| lease.window)
        .collect();
    let to_park: Vec<TaskWindow> = {
        let tasks = tasks();
        let space = tasks.as_ref().and_then(|tasks| tasks.get(task));
        held.into_iter()
            .filter(|window| {
                let id = window.window_id.as_deref().unwrap_or_default();
                space.is_none_or(|space| !space.kept.contains(id) && !space.parked.contains(window))
            })
            .collect()
    };
    let parked = to_park.into_iter().filter(park).collect();
    finish_parking(task, parked);
}

fn finish_parking(task: &str, parked: Vec<TaskWindow>) {
    let mut tasks = tasks();
    if let Some(space) = tasks.get_or_insert_with(HashMap::new).get_mut(task) {
        space.parking = false;
        space.parked.extend(parked);
    }
}

/// Brings back every window `task` parked. `kept`: the person took the task
/// over or brought it forward, so they stay out while it runs; else the
/// task ended and Hex forgets it. Blocks while it moves them (off the UI
/// thread).
pub fn show_task(task: &str, kept: bool) {
    let parked = {
        let mut tasks = tasks();
        let all = tasks.get_or_insert_with(HashMap::new);
        if kept {
            all.get_mut(task).map_or_else(Vec::new, |space| {
                let parked = std::mem::take(&mut space.parked);
                space
                    .kept
                    .extend(parked.iter().filter_map(|window| window.window_id.clone()));
                parked
            })
        } else {
            all.remove(task)
                .map(|space| space.parked)
                .unwrap_or_default()
        }
    };
    for window in parked {
        // Shown even if the setting went off meanwhile: it was moved away.
        if let Err(error) = show_now(&window) {
            tracing::warn!(
                error,
                app = window.app,
                "couldn't bring a parked window back"
            );
        }
    }
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
    let to = parking_spot(Os::HERE, &displays, preferred.as_deref())
        .ok_or("agent-desktop listed no displays")?;
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

/// How much of a parked window stays on screen on `os`: a point on macOS
/// and Linux; on Windows a window's frame includes an invisible resize
/// border of about 8 pixels, so a sliver of 16 keeps some of it showing.
pub fn sliver(os: Os) -> f64 {
    if os == Os::Windows { 16.0 } else { 1.0 }
}

/// Where a parked window's top-left corner goes: its [`sliver`] inside the
/// bottom-right corner of a display with no other display to its right or
/// below (else its bottom-left, then the main display's bottom-right), so
/// the window hangs off every screen but a sliver. The agents' display
/// (`preferred`, an id) is tried first, then the main one, so a parked
/// window's sliver stays off the person's working display when it can.
pub fn parking_spot(os: Os, displays: &[Display], preferred: Option<&str>) -> Option<(f64, f64)> {
    let inset = sliver(os);
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
            return Some((right - inset, bottom - inset));
        }
    }
    for display in &ordered {
        let (left, bottom) = (display.x, display.y + display.height);
        if free((left - 1.0, bottom - 1.0))
            && free((left + 1.0, bottom + 1.0))
            && free((left - 1.0, bottom + 1.0))
        {
            return Some((left - inset, bottom - inset));
        }
    }
    let main = ordered.first()?;
    Some((main.x + main.width - inset, main.y + main.height - inset))
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
        assert_eq!(parking_spot(Os::Mac, &pair, None), Some((1799.0, 1168.0)));
        // With the display above as the agents' display, its own free
        // corner: its bottom-right hangs past the laptop's right edge.
        assert_eq!(
            parking_spot(Os::Mac, &pair, Some("5")),
            Some((1955.0, -1.0))
        );
        // A taller display to the right of the main one: park on that one.
        let main = display(0.0, 0.0, 1800.0, 1200.0, true);
        let right = display(1800.0, 0.0, 1920.0, 1300.0, false);
        assert_eq!(
            parking_spot(Os::Mac, &[main.clone(), right], None),
            Some((3719.0, 1299.0))
        );
        // A shorter one: the main display's corner hangs below it.
        let short = display(1800.0, 0.0, 1920.0, 1080.0, false);
        assert_eq!(
            parking_spot(Os::Mac, &[main, short], None),
            Some((1799.0, 1199.0))
        );
        assert_eq!(parking_spot(Os::Mac, &[], None), None);
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
        let spot = parking_spot(Os::Mac, &all, None).unwrap();
        assert_eq!(spot, (-1.0, 1079.0));
        assert!(
            all.iter()
                .all(|display| { !display.contains((spot.0 + 2.0, spot.1 + 2.0)) })
        );
        // The agents' display (right) parks at its own corner.
        assert_eq!(
            parking_spot(Os::Mac, &all, Some("3")),
            Some((2719.0, 1023.0))
        );
        // The display above has no free corner of its own (main and the
        // side displays touch its bottom edge): the next free one.
        assert_eq!(parking_spot(Os::Mac, &all, Some("4")), Some((-1.0, 1079.0)));
    }

    #[test]
    fn windows_parks_past_its_invisible_border_and_wayland_not_at_all() {
        let main = display(0.0, 0.0, 1920.0, 1080.0, true);
        assert_eq!(
            parking_spot(Os::Windows, std::slice::from_ref(&main), None),
            Some((1904.0, 1064.0))
        );
        assert_eq!(
            parking_spot(Os::Linux, &[main], None),
            Some((1919.0, 1079.0))
        );
        assert!(moves_windows(Os::Mac, false) && moves_windows(Os::Windows, false));
        assert!(moves_windows(Os::Linux, false));
        assert!(!moves_windows(Os::Linux, true));
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
        park_held("c1");
        assert!(
            tasks()
                .as_ref()
                .is_none_or(|tasks| !tasks.contains_key("c1"))
        );
        show_task("c1", false);
    }
}
