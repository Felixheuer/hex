//! The desk in a task's guard (`guard::desk`): windows a quick turn opens
//! are handed to the desk, which Hex holds, and every task that may open
//! apps acts in desk windows as in its own; `hex_release_window` lets go.

use serde_json::{Map, Value, json};

use super::super::child::Desktop;
use super::super::desk::DESK;
use super::super::policy::RELEASE_TOOL;
use super::super::state::{Acquire, LeaseFile};
use super::apps::success;
use super::{Guard, refusal, same_app, text};
use crate::assistant::types::Lane;

/// What a quick turn's answer says of a window it handed to the desk.
pub(in crate::assistant::guard) const DESK_NOTE: &str = "It stays on the person's desk after this turn (parked out of their way, a live mini at the \
     orb), so later turns act in it directly; when the person is done with it (\"schließ …\"), \
     let go of it with hex_release_window.";

impl<D: Desktop> Guard<D> {
    /// Whether this task acts in desk windows: Background tasks, and Here
    /// tasks that may open apps.
    pub(super) fn reaches_desk(&self) -> bool {
        self.config.lane == Lane::Background || self.opens_apps()
    }

    /// The desk's windows, for `held` (when this task reaches them).
    pub(super) fn desk_leases(&self) -> Vec<LeaseFile> {
        if self.reaches_desk() {
            self.store.held_by(DESK)
        } else {
            Vec::new()
        }
    }

    /// Whether asking for a window came to one this task may use: its own,
    /// or the desk's.
    pub(super) fn usable(&self, acquired: &Acquire) -> bool {
        match acquired {
            Acquire::Acquired | Acquire::Held => true,
            Acquire::Taken(task) => task == DESK && self.reaches_desk(),
        }
    }

    /// Whether window `id` is free for this task: nobody's, its own, or
    /// the desk's.
    pub(super) fn free_for_this_task(&self, id: &str) -> bool {
        self.store.holder(id).is_none_or(|lease| {
            lease.task == self.config.task || (lease.task == DESK && self.reaches_desk())
        })
    }

    /// Hands window `id` (this task's) to the desk, in a quick turn with a
    /// desk to hand to; whether it is on the desk now.
    pub(super) fn to_desk(&mut self, id: &str) -> bool {
        let Some(desk) = self.desk else {
            return false;
        };
        if self.config.lane != Lane::Here {
            return false;
        }
        if self
            .store
            .holder(id)
            .is_some_and(|lease| lease.task == DESK)
        {
            return true;
        }
        let now = crate::desktop::events::now_ms();
        let handed = self.store.hand_over(id, &self.config.task, DESK, desk, now);
        if handed {
            tracing::info!(window = id, "a quick turn's window is on the desk now");
        }
        handed
    }

    /// [`Self::to_desk`] for each window in a `hex_open_app` answer's list;
    /// whether any went.
    pub(super) fn windows_to_desk(&mut self, windows: &[Value]) -> bool {
        let ids: Vec<String> = windows
            .iter()
            .filter_map(|window| window.get("id")?.as_str().map(str::to_owned))
            .collect();
        let mut any = false;
        for id in ids {
            any |= self.to_desk(&id);
        }
        any
    }

    /// `hex_release_window`: lets go of a desk window (by `window_id`), or
    /// every desk window of `app`; the window comes back on the screen.
    pub(super) fn release_tool(&mut self, args: &Map<String, Value>) -> Result<Value, Value> {
        let tool = RELEASE_TOOL;
        let id = text(args, "window_id");
        let app = text(args, "app");
        if id.is_none() && app.is_none() {
            return Err(refusal(
                tool,
                "INVALID_ARGS",
                "Name the window with window_id, or the app with app.",
            ));
        }
        let mut released = Vec::new();
        for lease in self.store.held_by(DESK) {
            let named = id.as_deref() == Some(lease.window_id.as_str())
                || app.as_deref().is_some_and(|app| same_app(app, &lease.app));
            if named {
                self.store.release(DESK, &lease.window_id);
                released.push(json!({"id": lease.window_id, "app_name": lease.app}));
            }
        }
        for lease in self.store.held_by(&self.config.task) {
            let named = id.as_deref() == Some(lease.window_id.as_str())
                || app.as_deref().is_some_and(|app| same_app(app, &lease.app));
            if named {
                self.store.release(&self.config.task, &lease.window_id);
                released.push(json!({"id": lease.window_id, "app_name": lease.app}));
            }
        }
        let note = if released.is_empty() {
            "Nothing of that is on the desk or yours."
        } else {
            "Let go: the window leaves the desk and comes back on the person's screen. To quit \
             the app as well, use desktop_close_app."
        };
        Ok(success(
            "release-window",
            json!({"released": released, "note": note}),
        ))
    }

    /// After `desktop_close_app` quit `app`: its desk windows are gone.
    pub(super) fn closed_app(&mut self, app: &str) {
        let _ = super::super::desk::release_app_in(&self.store, app);
    }
}

/// The tool's schema.
pub(super) fn tool() -> Value {
    json!({
        "name": RELEASE_TOOL,
        "description": "Hex: let go of an app window on the person's desk (the windows quick \
            turns opened stay there, parked, as live minis at the orb). Use it when the person \
            is done with it (\"schließ WhatsApp\", \"weg damit\"); the window comes back on their \
            screen. desktop_close_app quits the app instead.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "window_id": {"type": "string", "description": "The window's id (w-1234)"},
                "app": {"type": "string", "description": "Or every desk window of this app"}
            },
        },
    })
}
