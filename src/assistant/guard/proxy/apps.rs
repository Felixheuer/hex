//! Hex's own app tools: `hex_installed_apps` (what's installed, and what
//! runs) and `hex_open_app` (a launch that waits for the app's new window
//! and makes it the task's).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::super::child::{self, Desktop};
use super::super::policy::{self, INSTALLED_APPS_TOOL, OPEN_APP_TOOL};
use super::super::screen::App;
use super::{Guard, Launched, Look, refusal, text};
use crate::assistant::types::Lane;

/// How long `hex_open_app` waits for the app's new window, at most.
pub(super) const WINDOW_WAIT: Duration = Duration::from_secs(3);

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
        if self.config.lane == Lane::Here {
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
        let held_before: HashSet<String> = self
            .held()
            .into_iter()
            .map(|lease| lease.window_id)
            .collect();
        let _ = self.list(Look::Check);
        let mut launch = Map::new();
        launch.insert(
            "app".into(),
            json!(app.bundle_id.clone().unwrap_or_else(|| app.name.clone())),
        );
        if new_instance {
            launch.insert("no_attach".into(), json!(true));
        }
        let result = self.forward("desktop_launch", &launch)?;
        let Some(data) = child::data(&result) else {
            // agent-desktop's own failure, as it said it.
            return Ok(result);
        };
        let launched = Launched {
            app: app.name.clone(),
            pid: data.get("pid").and_then(Value::as_i64),
        };
        self.watch(&app.name);
        let waited = Instant::now();
        let mine = loop {
            let _ = self.list(Look::Launched(&launched));
            let mine: Vec<Value> = self
                .held()
                .into_iter()
                .filter(|lease| !held_before.contains(&lease.window_id))
                .map(|lease| json!({"id": lease.window_id, "app_name": lease.app}))
                .collect();
            if !mine.is_empty() || waited.elapsed() >= self.window_wait {
                break mine;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        // The notes `adopt` left say the same; the data has it.
        self.notes.clear();
        let note = match (mine.is_empty(), was_running && !new_instance) {
            (false, _) => "The new window is yours: act in it by its id.",
            (true, true) => {
                "The app was already running and opened no new window. Claim one of its free \
                 windows with desktop_claim_window, or call hex_open_app with new_instance: \
                 true for a fresh instance."
            }
            (true, false) => {
                "The app started but shows no window yet; wait with desktop_wait (event \
                 window-opened). A window it opens soon becomes yours."
            }
        };
        Ok(success(
            "open-app",
            json!({
                "app": app.name,
                "bundle_id": app.bundle_id,
                "path": app.path,
                "pid": launched.pid,
                "was_running": was_running,
                "new_instance": new_instance,
                "windows": mine,
                "note": note,
            }),
        ))
    }

    fn app_blocked(&mut self, app: &App) -> bool {
        self.blocked(&app.name) || app.bundle_id.as_ref().is_some_and(|id| self.blocked(id))
    }
}

fn success(command: &str, data: Value) -> Value {
    let envelope = json!({"version": "hex-guard", "ok": true, "command": command, "data": data});
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": false,
    })
}

/// The tools' schemas.
pub(super) fn tools(background: bool) -> Vec<Value> {
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
    if background {
        tools.push(json!({
            "name": OPEN_APP_TOOL,
            "description": "Hex: open an installed app without bringing it in front of the \
                person, wait for its new window, and make that window this task's. Prefer it to \
                desktop_launch. When the app already runs and opens no window, claim a free one \
                or ask for new_instance.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": {"type": "string", "description": "The app's name, as hex_installed_apps lists it"},
                    "bundle_id": {"type": "string", "description": "Or its bundle id"},
                    "new_instance": {"type": "boolean", "description": "Start a fresh instance even when it runs"}
                },
            },
        }));
    }
    tools
}
