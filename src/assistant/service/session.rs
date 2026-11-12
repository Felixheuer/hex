//! The screen frame and the agent-desktop multi-agent session around
//! Background work: turning the frame on and off as Background tasks come
//! and go, starting and ending the session, and each task's cursor (its
//! colour, and a label with its current step).

use std::sync::Arc;
use std::time::Instant;

use super::{Event, Lane, Runner, SESSION_WAIT, Session, State};

impl Runner {
    /// The desktop session a task joins: the Background work's, waiting a
    /// moment while it starts (a Here task doesn't wait).
    pub(super) fn session_for(&self, lane: Lane) -> Option<String> {
        let started = Instant::now();
        let mut state = self.state();
        loop {
            match &state.session {
                Session::On(id) => return Some(id.clone()),
                Session::Off => return None,
                Session::Starting if lane == Lane::Here => return None,
                Session::Starting => {
                    let left = SESSION_WAIT.saturating_sub(started.elapsed());
                    if left.is_zero() {
                        return None;
                    }
                    state = self
                        .session_changed
                        .wait_timeout(state, left)
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                }
            }
        }
    }

    /// The session to end now that no Background work is left and no task
    /// remains.
    pub(super) fn session_to_end(&self, state: &mut State) -> Option<String> {
        if state.background_on || !state.tasks.is_empty() {
            return None;
        }
        match std::mem::take(&mut state.session) {
            Session::On(id) => Some(id),
            other => {
                state.session = other;
                None
            }
        }
    }

    /// Turns the frame on or off as Background work starts or ends, with the
    /// desktop session around it (`Event::ModeChanged`).
    pub(super) fn refresh_mode(self: &Arc<Self>) {
        let (_, settings) = (self.hooks.settings)();
        // Working background jobs keep the computer awake; one waiting for
        // the person's answer doesn't.
        let working = self
            .state()
            .tasks
            .iter()
            .filter(|task| {
                task.lane() == Lane::Background
                    && task.status.state != crate::assistant::TaskState::NeedsInput
            })
            .count();
        crate::desktop::keep_awake::update(
            settings.keep_awake,
            settings.keep_display_awake,
            working,
        );
        let mut start = false;
        let (on, end) = {
            let mut state = self.state();
            let busy = state
                .tasks
                .iter()
                .any(|task| task.lane() == Lane::Background);
            if busy == state.background_on {
                return;
            }
            state.background_on = busy;
            if busy && settings.control_desktop && state.session == Session::Off {
                state.session = Session::Starting;
                start = true;
            }
            (busy, self.session_to_end(&mut state))
        };
        if let Some(session) = end {
            (self.hooks.end_session)(&session);
        }
        if start {
            let runner = Arc::clone(self);
            std::thread::spawn(move || {
                let started = (runner.hooks.start_session)();
                let end = {
                    let mut state = runner.state();
                    state.session = started.map_or(Session::Off, Session::On);
                    runner.session_changed.notify_all();
                    runner.session_to_end(&mut state)
                };
                if let Some(session) = end {
                    (runner.hooks.end_session)(&session);
                }
            });
        }
        self.emit(&Event::ModeChanged { on });
    }
}

/// The agent-desktop session around Background work: one multi-agent cursor
/// session, every task its own agent (and cursor) in it.
pub(super) mod desktop_session {
    use std::process::Stdio;

    use serde_json::Value;

    use crate::assistant::types::TaskWindow;

    fn agent_desktop(args: &[&str]) -> Option<Value> {
        let executable = crate::assistant::provider::agent_desktop()?;
        let output = crate::assistant::backends::agent_command(executable)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        serde_json::from_slice(&output.stdout).ok()
    }

    /// `agent-desktop session start --cursor --multi-agent`; its id.
    pub(in crate::assistant::service) fn start() -> Option<String> {
        let reply = agent_desktop(&[
            "session",
            "start",
            "--cursor",
            "--multi-agent",
            "--name",
            "Hex background tasks",
        ]);
        let id = reply
            .as_ref()
            .and_then(|reply| reply.pointer("/data/session_id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if id.is_none() {
            tracing::warn!(
                ?reply,
                "agent-desktop didn't start a session for background tasks"
            );
        }
        id
    }

    pub(in crate::assistant::service) fn end(session: &str) {
        let _ = agent_desktop(&["session", "end", session]);
    }

    /// Gives the agent's cursor its task colour (`palette`), its cursor
    /// image when the overlay drew one, and, when known, a label with what
    /// it is doing (at most six words).
    pub(in crate::assistant::service) fn style(
        session: &str,
        agent_id: &str,
        colour: u8,
        label: Option<&str>,
    ) {
        let mut args: Vec<String> = [
            "--session",
            session,
            "--agent-id",
            agent_id,
            "cursor-overlay",
            "enable",
        ]
        .map(str::to_owned)
        .into();
        args.extend(crate::assistant::palette::cursor_flags(colour));
        if let Some(image) = crate::rune_ui::assistant_overlay::cursor_image(colour) {
            args.extend(["--image".to_owned(), image.display().to_string()]);
        }
        if let Some(label) = label.filter(|label| !label.trim().is_empty()) {
            args.extend([
                "--label".to_owned(),
                label.to_owned(),
                "--max-words".to_owned(),
                "6".to_owned(),
            ]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = agent_desktop(&args);
    }

    /// Brings the window `task` holds (its lease, else the last window it
    /// named) forward: the person asked to take it over, so this one
    /// physical change is theirs.
    pub(in crate::assistant::service) fn bring_forward(task: &str, windows: &[TaskWindow]) {
        let leased: Vec<TaskWindow> = crate::assistant::guard::leases()
            .into_iter()
            .filter(|lease| lease.task == task)
            .map(|lease| lease.window)
            .collect();
        let window = leased
            .iter()
            .chain(windows.iter().rev())
            .find(|window| window.window_id.is_some())
            .or_else(|| leased.first().or(windows.last()))
            .cloned();
        let Some(window) = window else {
            return;
        };
        std::thread::spawn(move || {
            let reply = match &window.window_id {
                Some(id) => agent_desktop(&["focus-window", "--window-id", id]),
                None => agent_desktop(&["focus-window", "--app", &window.app]),
            };
            if reply.and_then(|reply| reply.get("ok").and_then(Value::as_bool)) != Some(true) {
                tracing::warn!(app = window.app, "couldn't bring the task's window forward");
            }
        });
    }
}
