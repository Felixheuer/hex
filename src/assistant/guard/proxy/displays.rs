//! Displays for a task: `hex_displays` (the layout, the person's display,
//! the agents' display), and Background windows kept on the agents'
//! display: each new one a task gets is moved there (cascading), and when
//! that display goes, the windows on it come back to the main one.

use serde_json::{Map, Value, json};

use super::super::child::{Desktop, Window};
use super::super::parking;
use super::super::policy::DISPLAYS_TOOL;
use super::super::screen::Display;
use super::Guard;
use super::apps::success;
use crate::assistant::computer;
use crate::assistant::types::Lane;

impl<D: Desktop> Guard<D> {
    pub(super) fn displays_tool(&mut self) -> Value {
        let displays = self.screen.displays();
        let agents = computer::agents_display(&displays, self.config.agent_display.as_deref());
        let person = displays.iter().find(|display| display.has_focus);
        success(
            "displays",
            json!({
                "displays": displays,
                "person_display": person.map(|display| &display.id),
                "agents_display": agents.map(|display| &display.id),
                "agents_display_setting": self.config.agent_display,
                "layout": computer::describe_displays(&displays, agents),
            }),
        )
    }

    /// Moves windows the task just got (`arrived`) onto the agents'
    /// display, cascading; with one display, parks them off the screen
    /// (`assistant.park_windows`, on by default) so a job works its apps out
    /// of the person's way, as Codex does: they stay operable through
    /// Accessibility and show live in the job's card at the orb.
    pub(super) fn place_arrivals(&mut self) {
        // The Here lane gets windows here only from apps it opens with full
        // control (`adopt`, `take_running`).
        let reaches = self.config.lane == Lane::Background || self.opens_apps();
        if !reaches || self.arrived.is_empty() {
            self.arrived.clear();
            return;
        }
        let arrived = std::mem::take(&mut self.arrived);
        let displays = self.screen.displays();
        // A display for the agents only when the person picked one; else
        // a job's windows park off the screen and show as live pictures on
        // its card (as Codex does), never on another monitor of theirs.
        let Some(agents) = picked_display(&displays, self.config.agent_display.as_deref()) else {
            if park_allowed() {
                for window in &arrived {
                    if let Some(spot) = park_spot(&displays) {
                        // Where it was first, so it always goes back.
                        self.note_parked(window, parking::How::OffScreen);
                        self.move_window(window, spot);
                        // Off the screen only where it still renders there.
                        self.measure_parked(window);
                    }
                }
            }
            return;
        };
        for window in arrived {
            let Some(frame) = window.bounds else {
                continue;
            };
            let already = computer::display_of(&displays, frame)
                .is_some_and(|display| display.id == agents.id);
            if !already {
                let index = self.store.leases().len() + self.placed.len();
                let to = computer::cascade(&agents, index, (frame.2, frame.3));
                self.move_window(&window, to);
            }
            self.placed.insert(window.id.clone(), agents.id.clone());
        }
    }

    /// Brings windows placed on a display that has gone back to the main
    /// display.
    pub(super) fn return_orphans(&mut self) {
        if self.placed.is_empty() {
            return;
        }
        let displays = self.screen.displays();
        if displays.is_empty() {
            return;
        }
        let gone: Vec<String> = self
            .placed
            .iter()
            .filter(|(_, display)| !displays.iter().any(|known| &known.id == *display))
            .map(|(window, _)| window.clone())
            .collect();
        if gone.is_empty() {
            return;
        }
        let Some(main) = displays
            .iter()
            .find(|display| display.is_primary)
            .or(displays.first())
            .cloned()
        else {
            return;
        };
        let held = self.held();
        let windows = self.screen.windows();
        for (index, id) in gone.into_iter().enumerate() {
            self.placed.remove(&id);
            let Some(lease) = held.iter().find(|lease| lease.window_id == id) else {
                continue;
            };
            let size = windows
                .iter()
                .find(|window| window.id == id)
                .and_then(|window| window.bounds)
                .map_or((800.0, 600.0), |frame| (frame.2, frame.3));
            let to = computer::cascade(&main, index, size);
            let window = Window {
                id: id.clone(),
                app: lease.app.clone(),
                title: String::new(),
                pid: None,
                focused: false,
                visible: true,
                minimized: false,
                bounds: None,
            };
            self.move_window(&window, to);
            self.notes.push(format!(
                "the display your window {id} was on is gone; it is on {} now.",
                main.name
            ));
        }
    }

    /// Notes in the parking store (`guard::parking`) that `window` waits
    /// `how` for this task, from where it is now (kept if it was parked
    /// before), so Hex puts it back when nobody holds it any more.
    pub(super) fn note_parked(&self, window: &Window, how: parking::How) {
        let now = crate::desktop::events::now_ms();
        parking::park(
            &self.store,
            &window.id,
            &window.app,
            window.bounds,
            how,
            &self.config.task,
            now,
        );
    }

    /// Where `window`, parked, comes on the screen when it's needed there:
    /// where it was before it was parked when that still shows, else
    /// `fallback`.
    pub(super) fn unparked_spot(&mut self, window: &Window, fallback: (f64, f64)) -> (f64, f64) {
        let Some(from) = parking::parked(&self.store, &window.id).and_then(|record| record.from)
        else {
            return fallback;
        };
        let displays: Vec<crate::assistant::space::Display> = self
            .screen
            .displays()
            .iter()
            .map(crate::assistant::space::Display::from)
            .collect();
        if crate::assistant::space::off_screen(from, &displays) {
            fallback
        } else {
            (from.0, from.1)
        }
    }

    pub(super) fn move_window(&mut self, window: &Window, (x, y): (f64, f64)) {
        let mut args = Map::new();
        args.insert("app".into(), json!(window.app));
        args.insert("window_id".into(), json!(window.id));
        args.insert("x".into(), json!(x));
        args.insert("y".into(), json!(y));
        if let Err(why) = self.desktop.call("desktop_move_window", &args) {
            tracing::warn!(
                window = window.id,
                why,
                "couldn't move a window to its display"
            );
        }
    }
}

/// The tool's schema.
pub(super) fn tool() -> Value {
    json!({
        "name": DISPLAYS_TOOL,
        "description": "Hex: the displays (id, name, frame and visible frame in global points \
            with the main display's top-left at 0,0 and y down, so a display above or left of \
            it has negative coordinates; scale), the one the person is working on, and the one \
            new windows of Background tasks go to. Read-only and fast.",
        "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}},
    })
}

/// The display the person picked for the agents (by id or name), if one
/// is picked and attached; "none" or empty is none.
fn picked_display(
    displays: &[crate::assistant::computer::Display],
    chosen: Option<&str>,
) -> Option<crate::assistant::computer::Display> {
    let chosen = chosen.map(|text| text.trim().to_lowercase())?;
    if chosen.is_empty() || chosen == "none" {
        return None;
    }
    computer::agents_display(displays, Some(&chosen))
        .filter(|display| {
            display.id.trim().to_lowercase() == chosen
                || display.name.trim().to_lowercase() == chosen
        })
        .cloned()
}

/// Whether windows a job opens may be parked off the screen
/// (`assistant.park_windows`), read once per guard process.
fn park_allowed() -> bool {
    // Tests never read the person's settings.
    if cfg!(test) {
        return false;
    }
    static ALLOWED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ALLOWED.get_or_init(|| {
        crate::app_settings::AppSettings::read()
            .map(|settings| settings.assistant.park_windows)
            .unwrap_or(true)
    })
}

/// Where a parked window's top-left corner goes on these displays
/// (`space::parking_spot`): a corner of a display with nothing beyond it,
/// so the window hangs off every screen but a sliver.
pub(super) fn park_spot(displays: &[crate::assistant::computer::Display]) -> Option<(f64, f64)> {
    let spaced: Vec<crate::assistant::space::Display> = displays
        .iter()
        .map(crate::assistant::space::Display::from)
        .collect();
    crate::assistant::space::parking_spot(&spaced, None)
}
