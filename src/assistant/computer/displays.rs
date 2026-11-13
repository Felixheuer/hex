//! The displays, as agent-desktop names them (`list-displays`' ids are
//! CoreGraphics display ids), in global points: top-left origin at the
//! primary display's top-left corner, y down, so a display above or left of
//! the primary one has negative coordinates. Which one the person works on
//! (the one holding the focused window), and which one Background tasks'
//! windows go to (the agents' display).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

/// `(x, y, width, height)` in global points.
pub type Frame = (f64, f64, f64, f64);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Display {
    /// agent-desktop's id (the CoreGraphics display id): "1", "5".
    pub id: String,
    /// What System Settings calls it: "Built-in Retina Display", "M27Q".
    pub name: String,
    pub frame: Frame,
    /// The frame less the menu bar and the Dock (the whole frame when only
    /// CoreGraphics could be asked).
    pub visible_frame: Frame,
    /// Backing pixels per point.
    pub scale: f64,
    pub is_primary: bool,
    /// The person is working here: it holds the focused window (without
    /// one, the pointer).
    pub has_focus: bool,
}

impl Display {
    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        let (left, top, width, height) = self.frame;
        x >= left && x < left + width && y >= top && y < top + height
    }

    fn area(&self) -> f64 {
        self.frame.2 * self.frame.3
    }
}

/// Every display, the primary first, with the one the person works on
/// marked. Cheap (CoreGraphics and Accessibility, a millisecond or so);
/// names come from AppKit on the main thread, else from a cache filled in
/// the background.
pub fn displays() -> Vec<Display> {
    #[cfg(target_os = "macos")]
    {
        let mut displays = macos::read();
        let focused = macos::focused_window_frame();
        let pointer = scrawl_capture::screens::pointer().map(|[x, y]| (x, y));
        mark_focus(&mut displays, focused, pointer);
        displays
    }
    #[cfg(target_os = "linux")]
    {
        super::linux::displays()
    }
    #[cfg(target_os = "windows")]
    {
        super::windows::displays()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

/// Marks the display holding most of `window` (the focused window) as the
/// person's, else the one under (or nearest) `pointer`, else the primary.
pub fn mark_focus(displays: &mut [Display], window: Option<Frame>, pointer: Option<(f64, f64)>) {
    for display in displays.iter_mut() {
        display.has_focus = false;
    }
    let by_window = window.and_then(|frame| index_of(displays, frame));
    let by_pointer = pointer.and_then(|point| index_of(displays, (point.0, point.1, 0.0, 0.0)));
    let primary = displays.iter().position(|display| display.is_primary);
    if let Some(index) = by_window
        .or(by_pointer)
        .or(primary)
        .or(if displays.is_empty() { None } else { Some(0) })
    {
        displays[index].has_focus = true;
    }
}

/// The display holding most of `frame` (a zero-size frame is a point), else
/// the nearest to its centre.
pub fn display_of(displays: &[Display], frame: Frame) -> Option<&Display> {
    index_of(displays, frame).map(|index| &displays[index])
}

fn index_of(displays: &[Display], (x, y, width, height): Frame) -> Option<usize> {
    let overlap = |display: &Display| {
        let (left, top, w, h) = display.frame;
        let across = (x + width).min(left + w) - x.max(left);
        let down = (y + height).min(top + h) - y.max(top);
        across.max(0.0) * down.max(0.0)
    };
    let best = displays
        .iter()
        .enumerate()
        .map(|(index, display)| (index, overlap(display)))
        .filter(|(_, area)| *area > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index);
    let centre = (x + width / 2.0, y + height / 2.0);
    best.or_else(|| displays.iter().position(|display| display.contains(centre)))
        .or_else(|| {
            let distance = |display: &Display| {
                let (left, top, w, h) = display.frame;
                let dx = (left - centre.0).max(centre.0 - (left + w)).max(0.0);
                let dy = (top - centre.1).max(centre.1 - (top + h)).max(0.0);
                dx * dx + dy * dy
            };
            displays
                .iter()
                .enumerate()
                .min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)))
                .map(|(index, _)| index)
        })
}

/// The display Background tasks' new windows go to: `chosen` (an id or a
/// name, from `assistant.agent_display`) while it is connected, else, with
/// two or more displays, one the person isn't working on (the primary if
/// free, else the largest). `None` with one display: windows stay where
/// they open.
pub fn agents_display<'a>(displays: &'a [Display], chosen: Option<&str>) -> Option<&'a Display> {
    if displays.len() < 2 {
        return None;
    }
    let key = |text: &str| text.trim().to_lowercase();
    if let Some(chosen) = chosen.map(key).filter(|chosen| !chosen.is_empty())
        && let Some(display) = displays
            .iter()
            .find(|display| key(&display.id) == chosen || key(&display.name) == chosen)
    {
        return Some(display);
    }
    let free: Vec<&Display> = displays
        .iter()
        .filter(|display| !display.has_focus)
        .collect();
    free.iter()
        .find(|display| display.is_primary)
        .or_else(|| free.iter().max_by(|a, b| a.area().total_cmp(&b.area())))
        .copied()
}

/// Where the `index`th window moved to `display` goes: down and right from
/// its visible frame's top-left corner, 32 points a step (eight steps, then
/// again), never so far that a `size` window would leave the display.
pub fn cascade(display: &Display, index: usize, (width, height): (f64, f64)) -> (f64, f64) {
    let (left, top, w, h) = display.visible_frame;
    let step = 32.0 * (index % 8) as f64;
    let x = (left + 40.0 + step).min(left + (w - width).max(0.0));
    let y = (top + 40.0 + step).min(top + (h - height).max(0.0));
    (x.round(), y.round())
}

/// Where `display` is from `other`'s point of view: "above and left of".
pub fn relation(display: &Display, other: &Display) -> String {
    let (x, y, w, h) = display.frame;
    let (ox, oy, ow, oh) = other.frame;
    let vertical = if y + h <= oy {
        Some("above")
    } else if y >= oy + oh {
        Some("below")
    } else {
        None
    };
    let horizontal = if x + w <= ox {
        Some("left of")
    } else if x >= ox + ow {
        Some("right of")
    } else if vertical.is_some() && x + w / 2.0 < ox + ow / 2.0 - 1.0 {
        Some("left of")
    } else if vertical.is_some() && x + w / 2.0 > ox + ow / 2.0 + 1.0 {
        Some("right of")
    } else {
        None
    };
    match (vertical, horizontal) {
        (Some(vertical), Some(horizontal)) => format!("{vertical} and {horizontal}"),
        (Some(vertical), None) => vertical.to_owned(),
        (None, Some(horizontal)) => horizontal.to_owned(),
        (None, None) => "overlapping".to_owned(),
    }
}

/// The layout for a prompt: the person's display, then where each other
/// one sits from it, and which display agents' windows go to.
pub fn describe(displays: &[Display], agents: Option<&Display>) -> Option<String> {
    let focus = displays.iter().find(|display| display.has_focus)?;
    let one = |display: &Display| {
        let (x, y, w, h) = display.frame;
        format!(
            "{} (display {}{}, {w}×{h} points at ({x}, {y}), scale {})",
            display.name,
            display.id,
            if display.is_primary { ", main" } else { "" },
            display.scale
        )
    };
    let mut text = format!("The person is working on {}.", one(focus));
    for other in displays.iter().filter(|display| display.id != focus.id) {
        text.push_str(&format!(
            " {} is {} it.",
            one(other),
            relation(other, focus)
        ));
    }
    if let Some(agents) = agents {
        text.push_str(&format!(
            " Background tasks' windows go to {} (display {}).",
            agents.name, agents.id
        ));
    }
    Some(text)
}

/// Display names by id, as last read from AppKit or `system_profiler`.
static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static ASKING: Mutex<bool> = Mutex::new(false);

fn name_of(id: &str) -> Option<String> {
    NAMES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()?
        .get(id)
        .cloned()
}

fn remember_names(names: impl IntoIterator<Item = (String, String)>) {
    NAMES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_insert_with(HashMap::new)
        .extend(names);
}

/// The names in `system_profiler SPDisplaysDataType -json`, by display id.
pub fn profiler_names(json: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .get("SPDisplaysDataType")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|gpu| gpu.get("spdisplays_ndrvs")?.as_array())
        .flatten()
        .filter_map(|display| {
            // It calls the built-in panel "Color LCD"; a person says built-in.
            let internal = display
                .get("spdisplays_connection_type")
                .and_then(serde_json::Value::as_str)
                == Some("spdisplays_internal");
            let name = match internal {
                true => "Built-in Display".to_owned(),
                false => display.get("_name")?.as_str()?.to_owned(),
            };
            Some((
                display.get("_spdisplays_displayID")?.as_str()?.to_owned(),
                name,
            ))
        })
        .collect()
}

/// Asks `system_profiler` for the names on a thread of its own (half a
/// second), once at a time.
fn learn_names() {
    {
        let mut asking = ASKING.lock().unwrap_or_else(|error| error.into_inner());
        if *asking {
            return;
        }
        *asking = true;
    }
    let spawned = std::thread::Builder::new()
        .name("hex-display-names".into())
        .spawn(|| {
            let output = crate::desktop::background_process::command("/usr/sbin/system_profiler")
                .args(["SPDisplaysDataType", "-json"])
                .output();
            if let Ok(output) = output {
                remember_names(profiler_names(&String::from_utf8_lossy(&output.stdout)));
            }
            *ASKING.lock().unwrap_or_else(|error| error.into_inner()) = false;
        });
    if spawned.is_err() {
        *ASKING.lock().unwrap_or_else(|error| error.into_inner()) = false;
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{Display, Frame};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayIsBuiltin(display: u32) -> u32;
    }

    pub fn read() -> Vec<Display> {
        let screens = scrawl_capture::screens::current();
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            names_from_appkit(mtm, &screens);
        }
        let mut missing = false;
        let displays = screens
            .iter()
            .filter(|screen| screen.id != 0)
            .map(|screen| {
                let id = screen.id.to_string();
                let name = super::name_of(&id).unwrap_or_else(|| {
                    missing = true;
                    // SAFETY: Any display id may be asked; an unknown one is false.
                    if unsafe { CGDisplayIsBuiltin(screen.id) } != 0 {
                        "Built-in Display".to_owned()
                    } else {
                        format!("Display {id}")
                    }
                });
                let [x, y, w, h] = screen.frame;
                let [vx, vy, vw, vh] = screen.visible;
                Display {
                    id,
                    name,
                    frame: (x, y, w, h),
                    visible_frame: (vx, vy, vw, vh),
                    scale: screen.scale,
                    is_primary: x == 0.0 && y == 0.0,
                    has_focus: false,
                }
            })
            .collect();
        if missing {
            super::learn_names();
        }
        displays
    }

    fn names_from_appkit(
        mtm: objc2::MainThreadMarker,
        screens: &[scrawl_capture::screens::Screen],
    ) {
        use objc2_app_kit::NSScreen;
        let all = NSScreen::screens(mtm);
        let Some(primary_height) = all.firstObject().map(|screen| screen.frame().size.height)
        else {
            return;
        };
        let mut names = Vec::new();
        for screen in all.iter() {
            let rect = screen.frame();
            let frame = scrawl_capture::screens::flip(
                [
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ],
                primary_height,
            );
            if let Some(found) = screens
                .iter()
                .find(|known| (0..4).all(|i| (known.frame[i] - frame[i]).abs() < 1.0))
            {
                names.push((found.id.to_string(), screen.localizedName().to_string()));
            }
        }
        super::remember_names(names);
    }

    /// The focused window's frame: the window the focused app says has
    /// focus, else its frontmost ordinary window.
    pub fn focused_window_frame() -> Option<Frame> {
        // The app in front from the workspace, not the system-wide
        // Accessibility element: asked from a worker thread while Agent-Desk itself
        // is in front, that request reached Agent-Desk's own UI tree and aborted
        // Agent-Desk (accesskit "RefCell already borrowed").
        let (pid, _, _) =
            objc2::rc::autoreleasepool(|_| scrawl_capture::capture::frontmost_external_app_info())?;
        if u32::try_from(pid).is_ok_and(|pid| pid == std::process::id()) {
            return None;
        }
        let number = crate::accessibility::focused_window_number(pid);
        let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
        let found = number
            .and_then(|number| windows.iter().find(|window| window.id == number))
            .or_else(|| {
                windows.iter().find(|window| {
                    window.pid == pid
                        && window.layer == 0
                        && window.alpha > 0.0
                        && window.bounds[2] > 1.0
                        && window.bounds[3] > 1.0
                })
            })?;
        let [x, y, w, h] = found.bounds;
        Some((x, y, w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: &str, frame: Frame, scale: f64) -> Display {
        Display {
            id: id.into(),
            name: format!("Display {id}"),
            frame,
            visible_frame: frame,
            scale,
            is_primary: frame.0 == 0.0 && frame.1 == 0.0,
            has_focus: false,
        }
    }

    /// This Mac: the built-in display (primary) and an M27Q above it and
    /// to the left, in negative coordinates.
    fn this_mac() -> Vec<Display> {
        let mut builtin = display("1", (0.0, 0.0, 1800.0, 1169.0), 2.0);
        builtin.name = "Built-in Retina Display".into();
        builtin.visible_frame = (0.0, 33.0, 1800.0, 1136.0);
        let mut m27q = display("5", (-604.0, -1440.0, 2560.0, 1440.0), 2.0);
        m27q.name = "M27Q".into();
        m27q.visible_frame = (-604.0, -1415.0, 2560.0, 1415.0);
        vec![builtin, m27q]
    }

    fn focus(displays: &[Display]) -> Option<&str> {
        displays
            .iter()
            .find(|display| display.has_focus)
            .map(|display| display.id.as_str())
    }

    fn id(display: Option<&Display>) -> Option<&str> {
        display.map(|display| display.id.as_str())
    }

    #[test]
    fn the_person_works_on_the_display_holding_the_focused_window() {
        let mut displays = this_mac();
        // A window on the M27Q, wholly in negative coordinates.
        let on_m27q = Some((-300.0, -1200.0, 900.0, 700.0));
        mark_focus(&mut displays, on_m27q, Some((100.0, 100.0)));
        assert_eq!(focus(&displays), Some("5"));
        // A window straddling both: the one holding more of it.
        mark_focus(&mut displays, Some((1000.0, -200.0, 600.0, 800.0)), None);
        assert_eq!(focus(&displays), Some("1"));
        // No focused window: the pointer's display.
        mark_focus(&mut displays, None, Some((-500.0, -10.0)));
        assert_eq!(focus(&displays), Some("5"));
        // A pointer in the gap right of the M27Q (x past 1956, y < 0): the
        // nearest display.
        mark_focus(&mut displays, None, Some((2100.0, -300.0)));
        assert_eq!(focus(&displays), Some("5"));
        // Nothing to go by: the primary.
        mark_focus(&mut displays, None, None);
        assert_eq!(focus(&displays), Some("1"));
    }

    #[test]
    fn the_agents_display_is_the_one_the_person_is_not_on() {
        let mut displays = this_mac();
        mark_focus(&mut displays, Some((100.0, 100.0, 800.0, 600.0)), None);
        assert_eq!(id(agents_display(&displays, None)), Some("5"));
        mark_focus(&mut displays, Some((-300.0, -1200.0, 800.0, 600.0)), None);
        assert_eq!(id(agents_display(&displays, None)), Some("1"));
        // Chosen by id or by name, even the person's own.
        assert_eq!(id(agents_display(&displays, Some("5"))), Some("5"));
        assert_eq!(id(agents_display(&displays, Some(" m27q "))), Some("5"));
        // A chosen display that's disconnected: automatic again.
        assert_eq!(id(agents_display(&displays, Some("7"))), Some("1"));
        // One display: windows stay where they open.
        assert_eq!(agents_display(&displays[..1], None), None);
    }

    #[test]
    fn three_displays_left_right_and_above_with_mixed_scales() {
        // A 1× display left, the 2× built-in, a 1× display right, a 2× one
        // above: frames are points whatever the scale.
        let mut displays = vec![
            display("1", (0.0, 0.0, 1512.0, 982.0), 2.0),
            display("2", (-1920.0, 0.0, 1920.0, 1080.0), 1.0),
            display("3", (1512.0, 0.0, 1280.0, 1024.0), 1.0),
            display("4", (-524.0, -1440.0, 2560.0, 1440.0), 2.0),
        ];
        let window = |frame| id(display_of(&displays, frame)).map(str::to_owned);
        assert_eq!(window((-1000.0, 100.0, 400.0, 300.0)).as_deref(), Some("2"));
        assert_eq!(window((2000.0, 500.0, 400.0, 300.0)).as_deref(), Some("3"));
        assert_eq!(window((0.0, -900.0, 400.0, 300.0)).as_deref(), Some("4"));
        assert_eq!(window((-5.0, -5.0, 0.0, 0.0)).as_deref(), Some("4"));
        assert_eq!(relation(&displays[1], &displays[0]), "left of");
        assert_eq!(relation(&displays[2], &displays[0]), "right of");
        assert_eq!(relation(&displays[3], &displays[0]), "above");
        // The person on the right display: the free primary takes agents.
        mark_focus(&mut displays, Some((2000.0, 500.0, 400.0, 300.0)), None);
        assert_eq!(id(agents_display(&displays, None)), Some("1"));
        // The person on the primary: the largest free display.
        mark_focus(&mut displays, Some((100.0, 100.0, 400.0, 300.0)), None);
        assert_eq!(id(agents_display(&displays, None)), Some("4"));
        // A window cascaded onto the 1× display stays inside it.
        let (x, y) = cascade(&displays[2], 3, (1000.0, 900.0));
        assert!(x >= 1512.0 && x + 1000.0 <= 2792.0 && y >= 0.0 && y + 900.0 <= 1024.0);
    }

    #[test]
    fn this_macs_layout_reads_as_a_person_would_say_it() {
        let mut displays = this_mac();
        mark_focus(&mut displays, Some((-300.0, -1200.0, 900.0, 700.0)), None);
        assert_eq!(relation(&displays[1], &displays[0]), "above and left of");
        assert_eq!(relation(&displays[0], &displays[1]), "below and right of");
        let agents = agents_display(&displays, None);
        assert_eq!(
            describe(&displays, agents).unwrap(),
            "The person is working on M27Q (display 5, 2560×1440 points at (-604, -1440), scale \
             2). Built-in Retina Display (display 1, main, 1800×1169 points at (0, 0), scale 2) \
             is below and right of it. Background tasks' windows go to Built-in Retina Display \
             (display 1)."
        );
    }

    #[test]
    fn windows_cascade_inside_the_visible_frame_even_at_negative_origins() {
        let displays = this_mac();
        let m27q = &displays[1];
        assert_eq!(cascade(m27q, 0, (800.0, 600.0)), (-564.0, -1375.0));
        assert_eq!(cascade(m27q, 1, (800.0, 600.0)), (-532.0, -1343.0));
        assert_eq!(
            cascade(m27q, 9, (800.0, 600.0)),
            cascade(m27q, 1, (800.0, 600.0))
        );
        // Too big to step: kept on the display.
        let (x, y) = cascade(m27q, 7, (2500.0, 1400.0));
        assert!((-604.0..=-544.0).contains(&x), "{x}");
        assert!(y >= -1415.0 && y + 1400.0 <= 0.0, "{y}");
    }

    #[test]
    fn names_are_read_from_system_profiler() {
        let json = r#"{"SPDisplaysDataType":[{"spdisplays_ndrvs":[
            {"_name":"Color LCD","_spdisplays_displayID":"1","spdisplays_connection_type":"spdisplays_internal"},
            {"_name":"M27Q","_spdisplays_displayID":"5"}]}]}"#;
        assert_eq!(
            profiler_names(json),
            [
                ("1".to_owned(), "Built-in Display".to_owned()),
                ("5".to_owned(), "M27Q".to_owned())
            ]
        );
        assert!(profiler_names("not json").is_empty());
    }

    /// Live, read-only: this Mac's displays as Agent-Desk reads them.
    #[cfg(target_os = "macos")]
    #[test]
    fn this_macs_displays_are_read() {
        let _ = displays();
        // Names come in the background on first use.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let started = std::time::Instant::now();
        let displays = displays();
        let took = started.elapsed();
        let agents = agents_display(&displays, None);
        eprintln!("displays() in {took:?}:");
        for display in &displays {
            eprintln!("  {display:?}");
        }
        eprintln!("agents' display: {:?}", id(agents));
        eprintln!("{}", describe(&displays, agents).unwrap_or_default());
        if displays.is_empty() {
            return;
        }
        assert_eq!(
            displays.iter().filter(|display| display.has_focus).count(),
            1
        );
        assert!(displays[0].is_primary);
    }
}
