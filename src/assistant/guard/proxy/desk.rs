//! The desk in a task's guard (`guard::desk`): windows a quick turn opens
//! are handed to the desk, which Agent-Desk holds, and every task that may open
//! apps acts in desk windows as in its own; `hex_release_window` lets go.

use serde_json::{Map, Value, json};

use super::super::child::Desktop;
use super::super::desk::{DESK, Lend, borrow, borrower};
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

    /// The desk's windows this task may use now, for `held`: every one for
    /// a quick turn that reaches the desk (it borrows each as it acts,
    /// `borrow_desk`); for a job, only those it has borrowed by claiming.
    pub(super) fn desk_leases(&self) -> Vec<LeaseFile> {
        if !self.reaches_desk() {
            return Vec::new();
        }
        let mut leases = self.store.held_by(DESK);
        if self.config.lane == Lane::Background {
            let now = crate::desktop::events::now_ms();
            leases.retain(|lease| {
                borrower(&self.store, &lease.window_id, now)
                    .is_some_and(|held| held.task == self.config.task)
            });
        }
        leases
    }

    /// Borrows desk window `id` for this task (`guard::desk::borrow`).
    fn lend(&self, id: &str) -> Lend {
        let now = crate::desktop::events::now_ms();
        let here = self.config.lane == Lane::Here;
        borrow(&self.store, id, &self.config.task, self.pid, here, now)
    }

    /// Whether asking for window `id` came to one this task may use: its
    /// own, or a desk window it could borrow (an explicit claim or opening
    /// its app; refused while another task has it, a job's while a quick
    /// turn has it).
    pub(super) fn usable(&mut self, id: &str, acquired: &Acquire) -> bool {
        match acquired {
            Acquire::Acquired | Acquire::Held => true,
            Acquire::Taken(task) => {
                task == DESK && self.reaches_desk() && self.lend(id) == Lend::Lent
            }
        }
    }

    /// Whether window `id` is free for this task: nobody's, its own, or a
    /// desk window no other task has borrowed (a quick turn's always is).
    pub(super) fn free_for_this_task(&self, id: &str) -> bool {
        self.store.holder(id).is_none_or(|lease| {
            if lease.task == self.config.task {
                return true;
            }
            if lease.task != DESK || !self.reaches_desk() {
                return false;
            }
            self.config.lane == Lane::Here
                || borrower(&self.store, id, crate::desktop::events::now_ms())
                    .is_none_or(|held| held.task == self.config.task)
        })
    }

    /// Before an action: each desk window it acts in is borrowed for this
    /// task (renewed when it has it), so two tasks never act in one at the
    /// same time; refused when another has it.
    pub(super) fn borrow_desk(
        &mut self,
        tool: &str,
        kind: super::Kind,
        resolved: &[super::Resolved],
    ) -> Result<(), Value> {
        if !kind.acts() || !self.reaches_desk() {
            return Ok(());
        }
        for window in resolved {
            let Some(id) = &window.id else {
                continue;
            };
            if self.store.holder(id).is_none_or(|lease| lease.task != DESK) {
                continue;
            }
            if let Lend::Busy { .. } = self.lend(id)
                && let Some(refused) = self.desk_refusal(tool, id)
            {
                return Err(refused);
            }
        }
        Ok(())
    }

    /// Why this task can't use desk window `id` now, if that's why: another
    /// task has it borrowed, or a job hasn't claimed it.
    pub(super) fn desk_refusal(&self, tool: &str, id: &str) -> Option<Value> {
        if !self.reaches_desk() || self.store.holder(id)?.task != DESK {
            return None;
        }
        let now = crate::desktop::events::now_ms();
        let message = match borrower(&self.store, id, now) {
            Some(held) if held.task == self.config.task => return None,
            Some(held) if held.here => format!(
                "The person's quick turn is using {id} on their desk right now. Work in another \
                 window, or wait with desktop_wait and claim it again with desktop_claim_window."
            ),
            Some(_) => format!(
                "Another task is using {id} on the person's desk right now. Work in another \
                 window, or wait with desktop_wait and claim it again with desktop_claim_window."
            ),
            None if self.config.lane == Lane::Here => return None,
            None => format!(
                "{id} is on the person's desk: claim it with desktop_claim_window before using \
                 it (one task at a time; refused while their quick turn uses it)."
            ),
        };
        Some(refusal(tool, "DESK_BUSY", &message))
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
                super::super::desk::return_borrow(&self.store, &lease.window_id);
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
        "description": "Agent-Desk: let go of an app window on the person's desk (the windows quick \
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
