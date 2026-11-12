//! Displays for a task: `hex_displays` (the layout, the person's display,
//! the agents' display), and Background windows kept on the agents'
//! display: each new one a task gets is moved there (cascading), and when
//! that display goes, the windows on it come back to the main one.

use serde_json::{Map, Value, json};

use super::super::child::{Desktop, Window};
use super::super::policy::DISPLAYS_TOOL;
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
    /// display, cascading; with one display they stay where they opened.
    pub(super) fn place_arrivals(&mut self) {
        if self.config.lane != Lane::Background || self.arrived.is_empty() {
            return;
        }
        let arrived = std::mem::take(&mut self.arrived);
        let displays = self.screen.displays();
        let Some(agents) =
            computer::agents_display(&displays, self.config.agent_display.as_deref()).cloned()
        else {
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

    fn move_window(&mut self, window: &Window, (x, y): (f64, f64)) {
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
