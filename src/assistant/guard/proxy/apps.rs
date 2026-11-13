//! Hex's own app tools: `hex_installed_apps` (what's installed, and what
//! runs) and `hex_open_app` (a launch that waits for the app's new window
//! and makes it the task's).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::super::child::{self, Desktop, Window};
use super::super::policy::{self, INSTALLED_APPS_TOOL, OPEN_APP_TOOL};
use super::super::screen::App;
use super::super::state::Acquire;
use super::{Guard, Launched, Look, refusal, same_app, text};
use crate::assistant::types::Lane;

/// How long `hex_open_app` waits for the app's new window, at most.
pub(super) const WINDOW_WAIT: Duration = Duration::from_secs(3);

/// Asks a running app to reopen (its main window comes back), without
/// bringing it in front: `open -g` sends the reopen event in the background.
pub(super) fn reopen(app: &App) {
    let mut open = std::process::Command::new("/usr/bin/open");
    open.arg("-g");
    match &app.bundle_id {
        Some(id) => open.args(["-b", id]),
        None => open.arg(&app.path),
    };
    let _ = open
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

impl<D: Desktop> Guard<D> {
    pub(super) fn installed_apps(&mut self, args: &Map<String, Value>) -> Value {
        let query = text(args, "query").map(|query| policy::app_key(&query));
        let running: HashSet<_> = self.screen.running_apps().into_iter().collect();
        let mut apps: Vec<App> = self
            .screen
            .installed_apps()
            .into_iter()
            .filter(|app| {
                query.as_ref().is_none_or(|query| {
                    policy::app_key(&app.name).contains(query.as_str())
                        || app
                            .bundle_id
                            .as_ref()
                            .is_some_and(|id| policy::app_key(id).contains(query.as_str()))
                })
            })
            .collect();
        apps.sort_by_key(|app| policy::app_key(&app.name));
        let listed: Vec<Value> = apps
            .iter()
            .map(|app| {
                let blocked = self.app_blocked(app);
                json!({
                    "name": app.name,
                    "bundle_id": app.bundle_id,
                    "path": app.path,
                    "running": running.contains(&app.path),
                    "blocked": blocked,
                })
            })
            .collect();
        success("installed-apps", json!({ "apps": listed }))
    }

    pub(super) fn open_app(&mut self, args: &Map<String, Value>) -> Result<Value, Value> {
        let tool = OPEN_APP_TOOL;
        if self.config.lane == Lane::Here && !self.opens_apps() {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "This task was asked about the window in front and acts only there; opening \
                 apps is for Background tasks.",
            ));
        }
        let wanted = text(args, "bundle_id")
            .or_else(|| text(args, "name"))
            .ok_or_else(|| refusal(tool, "INVALID_ARGS", "Name the app with name or bundle_id."))?;
        let new_instance = args.get("new_instance").and_then(Value::as_bool) == Some(true);
        let key = policy::app_key(&wanted);
        let app = self
            .screen
            .installed_apps()
            .into_iter()
            .find(|app| {
                policy::app_key(&app.name) == key
                    || app
                        .bundle_id
                        .as_ref()
                        .is_some_and(|id| policy::app_key(id) == key)
            })
            .ok_or_else(|| {
                refusal(
                    tool,
                    "APP_NOT_FOUND",
                    &format!("No app “{wanted}” is installed; {INSTALLED_APPS_TOOL} lists them."),
                )
            })?;
        if self.app_blocked(&app) {
            return Err(self.blocked_refusal(tool, &app.name));
        }
        let was_running = self.screen.running_apps().contains(&app.path);
        // What is on screen now, so what opens next is new.
        let _ = self.list(Look::Check);
        if was_running {
            return Ok(self.take_running(&app));
        }
        let held_before: HashSet<String> = self
            .held()
            .into_iter()
            .map(|lease| lease.window_id)
            .collect();
        let mut launch = Map::new();
        launch.insert(
            "app".into(),
            json!(app.bundle_id.clone().unwrap_or_else(|| app.name.clone())),
        );
        // Only an app that isn't running starts here (a running one is
        // taken above): a separate instance when asked.
        if new_instance {
            launch.insert("no_attach".into(), json!(true));
        }
        // The person's app keeps the front (`front`): noted before.
        let before = super::front::frontmost();
        let result = self.forward("desktop_launch", &launch)?;
        let Some(data) = child::data(&result) else {
            // agent-desktop's own failure, as it said it.
            return Ok(result);
        };
        let launched = Launched {
            app: app.name.clone(),
            pid: data.get("pid").and_then(Value::as_i64),
        };
        let mine = self.wait_for_window(&launched, &held_before);
        super::front::keep_front(before, launched.pid);
        // Out of the person's way now, before the task acts in it.
        self.place_arrivals();
        // The notes `adopt` left say the same; the data has it.
        self.notes.clear();
        let note = if mine.is_empty() {
            "The app started but shows no window yet; wait with desktop_wait (event \
             window-opened). A window it opens soon becomes yours."
        } else {
            "The new window is yours: act in it by its id."
        };
        Ok(opened(&app, launched.pid, false, new_instance, mine, note))
    }

    /// The new windows `launched` shows within `window_wait` that are the
    /// task's now (not among `held_before`).
    fn wait_for_window(
        &mut self,
        launched: &Launched,
        held_before: &HashSet<String>,
    ) -> Vec<Value> {
        self.watch(&launched.app);
        let waited = Instant::now();
        loop {
            let _ = self.list(Look::Launched(launched));
            let mine: Vec<Value> = self
                .held()
                .into_iter()
                .filter(|lease| !held_before.contains(&lease.window_id))
                .map(|lease| json!({"id": lease.window_id, "app_name": lease.app, "on_screen": true}))
                .collect();
            if !mine.is_empty() || waited.elapsed() >= self.window_wait {
                return mine;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// `hex_open_app` for an app that already runs: never a second
    /// instance. Its window on this screen becomes the task's at once (the
    /// one in front first: the person is in that app); without one, a
    /// window it has elsewhere (another desktop, minimized, hidden) does;
    /// an app with no window at all is asked to reopen one, in the
    /// background. A window that wasn't in front goes out of the person's
    /// way like any the task opens (`place_arrivals`).
    fn take_running(&mut self, app: &App) -> Value {
        let pids = self.screen.app_pids(&app.path);
        let full = self.config.front_window;
        let of_app = |window: &Window| {
            window.pid.is_some_and(|pid| pids.contains(&pid)) || same_app(&window.app, &app.name)
        };
        let mut on_screen: Vec<Window> = self
            .list(Look::Check)
            .unwrap_or_default()
            .into_iter()
            // The window in front is the person's unless they gave full
            // control.
            .filter(|window| window.visible && of_app(window) && (full || !window.focused))
            .collect();
        // Stable: front to back after the one in front.
        on_screen.sort_by_key(|window| !window.focused);
        let off_screen: Vec<Window> = self
            .screen
            .off_screen_windows()
            .into_iter()
            .filter(|window| of_app(window) && !on_screen.iter().any(|on| on.id == window.id))
            .collect();
        let pid = pids.first().copied().or_else(|| {
            on_screen
                .iter()
                .chain(&off_screen)
                .find_map(|window| window.pid)
        });
        self.watch(&app.name);
        for window in on_screen.iter().chain(&off_screen) {
            let now = crate::desktop::events::now_ms();
            let taken =
                self.store
                    .acquire(&self.config.task, self.pid, &window.id, &window.app, now);
            if matches!(taken, Acquire::Taken(_)) {
                continue;
            }
            if taken == Acquire::Acquired && window.visible && !window.focused {
                self.arrived.push(window.clone());
            }
            self.place_arrivals();
            let note = if window.focused {
                "The app was already running and in front: its window is yours now (the person \
                 is looking at it). Act in it by its id."
            } else if window.visible {
                "The app was already running: its window is yours now, out of the person's way \
                 (they see it live on your card). Act in it by its id."
            } else {
                "The app was already running with its window off this screen (on another \
                 desktop, minimized, or the app hidden): it is yours now. Act in it by its id; \
                 if a snapshot finds nothing, desktop_restore brings a minimized one back."
            };
            let windows = vec![json!({
                "id": window.id,
                "app_name": window.app,
                "title": window.title,
                "on_screen": window.visible,
            })];
            return opened(app, pid, true, false, windows, note);
        }
        let held_before: HashSet<String> = self
            .held()
            .into_iter()
            .map(|lease| lease.window_id)
            .collect();
        let taken_by_others = !on_screen.is_empty() || !off_screen.is_empty();
        let mut mine = Vec::new();
        if !taken_by_others {
            // Running without a window (its last one closed): it reopens
            // one, in the background.
            let before = super::front::frontmost();
            (self.reopen)(app);
            let launched = Launched {
                app: app.name.clone(),
                pid,
            };
            mine = self.wait_for_window(&launched, &held_before);
            super::front::keep_front(before, pid);
            self.place_arrivals();
            self.notes.clear();
        }
        let note = match (mine.is_empty(), taken_by_others) {
            (false, _) => {
                "The app was already running without a window; it opened one, which is yours: \
                 act in it by its id."
            }
            (true, true) => {
                "The app is running, but each of its windows belongs to another task. Never \
                 start a second instance: wait for that task, or ask it with hex_tell_job."
            }
            (true, false) => {
                "The app is running but showed no window after being asked to reopen one; wait \
                 with desktop_wait (event window-opened). Never start a second instance."
            }
        };
        opened(app, pid, true, false, mine, note)
    }

    fn app_blocked(&mut self, app: &App) -> bool {
        self.blocked(&app.name) || app.bundle_id.as_ref().is_some_and(|id| self.blocked(id))
    }
}

/// `hex_open_app`'s answer.
fn opened(
    app: &App,
    pid: Option<i64>,
    was_running: bool,
    new_instance: bool,
    windows: Vec<Value>,
    note: &str,
) -> Value {
    success(
        "open-app",
        json!({
            "app": app.name,
            "bundle_id": app.bundle_id,
            "path": app.path,
            "pid": pid,
            "was_running": was_running,
            "new_instance": new_instance,
            "windows": windows,
            "note": note,
        }),
    )
}

pub(super) fn success(command: &str, data: Value) -> Value {
    let envelope = json!({"version": "hex-guard", "ok": true, "command": command, "data": data});
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": false,
    })
}

/// The tools' schemas.
pub(super) fn tools(open: bool) -> Vec<Value> {
    let mut tools = vec![json!({
        "name": INSTALLED_APPS_TOOL,
        "description": "Hex: the apps installed on this Mac, each with its name, bundle id, path, \
            whether it is running, and whether Hex blocks it. Read-only and fast.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {"type": "string", "description": "Only apps whose name or bundle id contains this"}
            },
        },
    })];
    if open {
        tools.push(json!({
            "name": OPEN_APP_TOOL,
            "description": "Hex: open or switch to an installed app and make its window this \
                task's, without covering the person's window. Prefer it to desktop_launch. An app \
                that isn't running starts and its new window is yours. An app that already runs \
                is never started again: its window (on this screen, else on another desktop or \
                minimized) becomes yours at once, and one without a window is asked to reopen \
                one. Answers with the window's id.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": {"type": "string", "description": "The app's name, as hex_installed_apps lists it"},
                    "bundle_id": {"type": "string", "description": "Or its bundle id"},
                    "new_instance": {"type": "boolean", "description": "Start a separate instance; ignored when the app already runs (most apps refuse a second one)"}
                },
            },
        }));
    }
    tools
}
