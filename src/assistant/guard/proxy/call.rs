//! One call through the guard: `Guard::call` checks a tool call against
//! the task's lane, leases, and pause, takes the real hands when it acts
//! with them, forwards it to the desktop driver, and adds Agent-Desk's notes to
//! the result.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::*;

impl<D: Desktop> Guard<D> {
    /// Checks and forwards one call; the MCP `CallToolResult` to return.
    pub fn call(&mut self, tool: &str, args: Map<String, Value>) -> Value {
        let started = Instant::now();
        self.notes.clear();
        self.forwarding = Duration::ZERO;
        self.notice_resume();
        self.refresh_here_window();
        let mut args = args;
        // A reader leaves the guard's upkeep to the guard.
        if self.config.lane == Lane::Background && !self.reader {
            self.return_orphans();
        }
        if !self.reader {
            self.borrow_expired();
        }
        let outcome = self.run(tool, &mut args);
        self.place_arrivals();
        tracing::debug!(
            tool,
            guard_ms = started
                .elapsed()
                .saturating_sub(self.forwarding)
                .as_secs_f64()
                * 1000.0,
            forward_ms = self.forwarding.as_secs_f64() * 1000.0,
            refused = outcome.is_err(),
            "guarded call"
        );
        let forwarded = outcome.is_ok();
        let mut result = match outcome {
            Ok(result) | Err(result) => result,
        };
        if forwarded && self.note_pending {
            self.note_pending = false;
            self.notes
                .push(RESUMED.replace(" (The action you asked for was not sent.)", ""));
        }
        if !self.notes.is_empty()
            && let Some(content) = result.get_mut("content").and_then(Value::as_array_mut)
        {
            for note in self.notes.drain(..) {
                content.push(json!({"type": "text", "text": format!("Agent-Desk: {note}")}));
            }
        }
        result
    }

    pub(super) fn run(
        &mut self,
        tool: &str,
        args: &mut Map<String, Value>,
    ) -> Result<Value, Value> {
        policy::lean_snapshot(tool, args);
        Self::want_bounds(tool, args);
        let kind = policy::kind(tool, args);
        if self.reader && !kind.reads_alone() {
            return Err(failure("Agent-Desk: this call isn't one a reader runs."));
        }
        if let Some(app) = text(args, "app")
            && self.blocked(&app)
        {
            return Err(self.blocked_refusal(tool, &app));
        }
        if tool.starts_with("desktop_") && kind.needs_desktop() {
            self.running_name(args);
        }
        // A locked screen stops every desktop step; say so instead of
        // failing silently (a sleeping display is woken on the way).
        if kind.needs_desktop()
            && crate::desktop::screen_lock::desktop_state()
                == crate::desktop::screen_lock::DesktopState::Locked
        {
            return Err(refusal(
                tool,
                "SCREEN_LOCKED",
                "The Mac's screen is locked, so no desktop step can run until the person unlocks it. Do every part that needs no desktop now (browser tools, files, shell); if desktop steps are still left, ask the person once: start your reply with `HEX_QUESTION:` and say the screen is locked and what is waiting for it.",
            ));
        }
        if self.config.lane == Lane::Here {
            self.ensure_here();
        }
        if self.config.observe_only
            && !matches!(
                kind,
                Kind::Free
                    | Kind::Observe
                    | Kind::InstalledApps
                    | Kind::AppCapabilities
                    | Kind::Displays
                    | Kind::ReadWindow
                    | Kind::WindowShot
            )
        {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "This is a check: look and read only, change nothing.",
            ));
        }
        match kind {
            Kind::Refused(why) => return Err(refusal(tool, "POLICY_DENIED", why)),
            Kind::Claim => return self.claim(tool, args),
            Kind::InstalledApps => return Ok(self.installed_apps(args)),
            Kind::Displays => return Ok(self.displays_tool()),
            Kind::Release => {
                if !self.reaches_desk() {
                    return Err(refusal(
                        tool,
                        "POLICY_DENIED",
                        "This task acts only in the window that was in front when it was asked.",
                    ));
                }
                return self.release_tool(args);
            }
            Kind::ReadWindow => return self.read_window(tool, args),
            Kind::WindowShot => return self.window_shot(tool, args),
            Kind::WindowInput => return self.window_input(tool, args),
            Kind::AppCapabilities => return self.app_capabilities(tool, args),
            Kind::Clipboard
                if self.config.lane == Lane::Background && !self.config.physical_input =>
            {
                return Err(refusal(
                    tool,
                    "POLICY_DENIED",
                    "The clipboard is shared with the person; a Background task puts text in its \
                     window with desktop_type or desktop_set_value instead.",
                ));
            }
            Kind::Clipboard => return self.forward(tool, args),
            Kind::Free => {
                let result = self.forward(tool, args)?;
                let background = self.config.lane == Lane::Background;
                if background && tool == "desktop_wait" && args.contains_key("event") {
                    let _ = self.list(Look::AfterAction);
                }
                if tool == "desktop_list_windows" {
                    self.windows_note();
                }
                return Ok(result);
            }
            _ => {}
        }
        if kind.acts() {
            self.gate(tool)?;
        }
        if kind == Kind::Physical && !self.config.physical_input {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "Real pointer and keyboard input isn't allowed for this task; act through refs \
                 (click, type, set_value, select, scroll, press with app).",
            ));
        }
        if self.config.lane == Lane::Here && matches!(kind, Kind::Launch) && !self.opens_apps() {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "This task was asked about the window in front and acts only there; opening \
                 apps is for Background tasks.",
            ));
        }
        if kind == Kind::Launch {
            return self.launch(tool, args);
        }
        if kind == Kind::OpenApp {
            return self.open_app(args);
        }
        if kind == Kind::AppAction {
            if self.config.lane == Lane::Here && !self.opens_apps() {
                return Err(refusal(
                    tool,
                    "POLICY_DENIED",
                    "This task was asked about the window in front and acts only there.",
                ));
            }
            return self.app_action(tool, args);
        }
        if kind == Kind::FocusWindow
            && self.config.lane == Lane::Background
            && !self.config.front_window
        {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "Focusing brings a window in front of the person; Background tasks work without \
                 it.",
            ));
        }
        let targets =
            policy::targets(tool, args).map_err(|why| refusal(tool, "POLICY_DENIED", &why))?;
        let mut resolved = Vec::new();
        for target in targets {
            resolved.extend(self.resolve(tool, kind, target, args)?);
        }
        let held = self.held();
        for window in &resolved {
            self.check(tool, kind, window, &held)?;
        }
        self.borrow_desk(tool, kind, &resolved)?;
        self.secure_read(tool, args)?;
        if kind.acts() {
            self.consent(tool, args, &resolved)?;
            if self.config.lane == Lane::Background || self.opens_apps() {
                for window in &resolved {
                    self.watch(&window.app);
                }
            }
            // What was on screen before, so what its action opens is new.
            let _ = self.list(Look::Check);
        }
        if kind.acts() {
            self.borrow_step(&resolved);
        }
        if kind == Kind::FocusWindow {
            let result = self.focus(tool, args, &resolved)?;
            let _ = self.list(Look::AfterAction);
            return Ok(result);
        }
        let hands = if kind.acts() && self.config.physical_input {
            Some(self.take_hands(tool)?)
        } else {
            None
        };
        let result = self.forward(tool, args);
        drop(hands);
        let mut result = result?;
        self.learn_and_redact(&mut result);
        self.learn_frames(&result);
        self.record(&result, &resolved);
        if kind.acts() {
            result = self.ax_fallback(tool, args, &resolved, result);
            self.count_failure(&resolved, &result);
            if kind == Kind::CloseApp
                && child::data(&result).is_some()
                && let Some(window) = resolved.first()
            {
                let app = window.app.clone();
                self.closed_app(&app);
            }
            let _ = self.list(Look::AfterAction);
            self.borrow_done();
        }
        Ok(result)
    }

    /// The real pointer and keyboard for this action, one task at a time
    /// (a second task's action would move the same pointer): waits a while
    /// for another task's action to end, else refuses as busy.
    pub(super) fn take_hands(&self, tool: &str) -> Result<super::super::state::HandsOn, Value> {
        self.store
            .take_hands(&self.config.task, self.pid, HANDS_WAIT)
            .map_err(|_| {
                refusal(
                    tool,
                    "BUSY",
                    "Another task is using the real mouse and keyboard right now. Try again in a \
                     moment, or do it with a headless action.",
                )
            })
    }

    pub(super) fn forward(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, Value> {
        let started = Instant::now();
        let result = self
            .desktop
            .call(tool, args)
            .map_err(|why| failure(&format!("Cua Driver: {why}")));
        self.forwarding += started.elapsed();
        result
    }
}

impl<D: Desktop> Guard<D> {
    /// Waits while the task is paused; an action that waited, or the first
    /// after a resume, is not sent: the window may have changed.
    pub(super) fn gate(&mut self, tool: &str) -> Result<(), Value> {
        if self.store.paused(&self.config.task) {
            // Taken over: a window brought forward is the person's now.
            self.hand_over_borrowed();
            while self.store.paused(&self.config.task) {
                std::thread::sleep(self.poll);
            }
            self.resumed = self.store.resumed(&self.config.task);
            self.note_pending = false;
            return Err(resumed_refusal(tool));
        }
        if self.note_pending {
            self.note_pending = false;
            return Err(resumed_refusal(tool));
        }
        Ok(())
    }

    pub(super) fn notice_resume(&mut self) {
        let resumed = self.store.resumed(&self.config.task);
        if resumed != self.resumed {
            self.resumed = resumed;
            self.note_pending = true;
        }
    }
}

impl<D: Desktop> Guard<D> {
    /// Names the app in `args["app"]` exactly as it runs: a name the agent
    /// typed ("WhatsApp") becomes the running app's own ("\u{200e}WhatsApp",
    /// with its invisible mark), so the driver finds it.
    pub(super) fn running_name(&mut self, args: &mut Map<String, Value>) {
        let Some(app) = text(args, "app") else {
            return;
        };
        let windows = self.list(Look::Check).unwrap_or_default();
        let real = windows
            .iter()
            .find(|window| window.visible && same_app(&window.app, &app))
            .or_else(|| windows.iter().find(|window| same_app(&window.app, &app)))
            .map(|window| window.app.clone());
        if let Some(real) = real
            && real != app
        {
            args.insert("app".into(), Value::String(real));
        }
    }
}
