//! Agent-Desk's fast actions for a task (`assistant::actions`): `hex_app_action`
//! (a URL or a script for any app), `hex_app_capabilities` (what an app
//! offers, from the app itself), and `hex_read_window` (a window's text
//! from its picture), checked like every other action: blocked apps are
//! refused, a consequential script needs the person's yes (`autonomy`),
//! and a window an action opens is the task's (and, in a quick turn, the
//! desk's), parked out of the person's way.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value, json};

use super::super::child::Desktop;
use super::super::policy::{APP_ACTION_TOOL, APP_CAPABILITIES_TOOL, READ_WINDOW_TOOL, Target};
use super::apps::success;
use super::{Guard, Launched, Look, Resolved, failure, refusal, text};
use crate::assistant::actions::{app_action, ocr};
use crate::assistant::autonomy::{self, Autonomy};

/// Most lines `hex_read_window` returns.
const MAX_LINES: usize = 600;

impl<D: Desktop> Guard<D> {
    pub(super) fn app_action(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, Value> {
        let app = text(args, "app").unwrap_or_default();
        let intent = text(args, "intent").unwrap_or_default();
        let values: BTreeMap<String, String> = args
            .get("params")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| {
                let value = match value {
                    Value::String(text) => text.clone(),
                    Value::Number(number) => number.to_string(),
                    Value::Bool(flag) => flag.to_string(),
                    _ => return None,
                };
                Some((key.clone(), value))
            })
            .collect();
        let installed = self.installed_app(&app);
        let (name, bundle_id) = installed
            .as_ref()
            .map(|found| (found.name.clone(), found.bundle_id.clone()))
            .unwrap_or_else(|| (app.clone(), None));
        let (recipe, command) =
            match app_action::build(&name, bundle_id.as_deref(), &intent, &values) {
                Some(built) => built.map_err(|why| refusal(tool, "INVALID_ARGS", &why))?,
                None => {
                    return Err(refusal(
                        tool,
                        "UNKNOWN_INTENT",
                        &format!(
                            "hex_app_action has no intent {intent}; every app takes open_url \
                             (params.url) and script (params.script, AppleScript). Find what \
                             {app} offers for them with hex_app_capabilities."
                        ),
                    ));
                }
            };
        let recipe = &recipe;
        self.vet_action(tool, recipe, installed.as_ref())?;
        // An action that shows a window: the app's window comes here first
        // (from another desktop if need be) and becomes the task's.
        let running = installed
            .as_ref()
            .is_some_and(|app| self.screen.running_apps().contains(&app.path));
        let shows = recipe.opens_window;
        let mut windows = Vec::new();
        let mut desk = false;
        if shows
            && running
            && let Some(app) = &installed
        {
            let taken = self.take_running(app);
            if let Some(data) = super::super::child::data(&taken) {
                windows = data["windows"].as_array().cloned().unwrap_or_default();
            }
            desk = windows
                .iter()
                .filter_map(|window| window["id"].as_str())
                .any(|id| {
                    self.store
                        .holder(id)
                        .is_some_and(|lease| lease.task == super::super::desk::DESK)
                });
            self.notes.clear();
        }
        let held_before: HashSet<String> = self
            .held()
            .into_iter()
            .map(|lease| lease.window_id)
            .collect();
        let _ = self.list(Look::Check);
        let before = super::front::frontmost();
        let output = (self.run_action)(&command).map_err(|why| {
            failure(&format!(
                "{} {} didn't work: {}",
                recipe.app,
                recipe.intent,
                if why.is_empty() {
                    "no reason given"
                } else {
                    &why
                }
            ))
        })?;
        if shows && !running {
            // It started: its new window is the task's.
            let launched = Launched {
                app: recipe.app.clone(),
                pid: None,
            };
            windows = self.wait_for_window(&launched, &held_before);
            self.place_arrivals();
            desk = self.windows_to_desk(&windows);
            self.notes.clear();
        }
        if let Some(app) = &installed {
            let pid = self.screen.app_pids(&app.path).first().copied();
            super::front::keep_front(before, pid);
        }
        self.watch(&recipe.app);
        let mut note = "Done.".to_owned();
        if !windows.is_empty() {
            note.push_str(" Its window is yours: act in it by its id (check with a snapshot).");
        }
        if desk {
            note = format!("{note} {}", super::DESK_NOTE);
        }
        Ok(success(
            "app-action",
            json!({
                "app": recipe.app,
                "intent": recipe.intent,
                "output": output,
                "windows": windows,
                "note": note,
            }),
        ))
    }

    /// The checks an app action passes before it runs, the same for every
    /// app: no blocked app among the ones it addresses (the one it is
    /// wrapped in and every other its script names); outside YOLO nothing
    /// that runs code beyond the apps' dictionaries (a shell, another
    /// script, a raw Apple event, a program, or an app that opens shell
    /// scripts, such as a terminal); and a consequential script waits for
    /// the person's yes (`autonomy::Gate`).
    fn vet_action(
        &mut self,
        tool: &str,
        recipe: &app_action::Action,
        installed: Option<&super::super::screen::App>,
    ) -> Result<(), Value> {
        let mut apps: Vec<(String, Option<String>, Option<std::path::PathBuf>)> = vec![(
            recipe.app.clone(),
            recipe.bundle_id.clone(),
            installed.map(|app| app.path.clone()),
        )];
        for target in &recipe.targets {
            // A path names the app by its bundle's name.
            let key = if target.contains('/') {
                std::path::Path::new(target.trim_end_matches('/'))
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| target.clone())
            } else {
                target.clone()
            };
            apps.push(match self.installed_app(&key) {
                Some(app) => (app.name, app.bundle_id, Some(app.path)),
                None => (key, None, None),
            });
        }
        for (name, bundle_id, _) in &apps {
            if self.blocked(name) || bundle_id.as_deref().is_some_and(|id| self.blocked(id)) {
                return Err(self.blocked_refusal(tool, name));
            }
        }
        let mode = self.config.autonomy;
        if mode != Autonomy::Yolo {
            let runs = recipe.runs_code.clone().or_else(|| {
                apps.iter()
                    .find(|(_, _, path)| path.as_deref().is_some_and(app_action::vet::runs_shell))
                    .map(|(name, _, _)| {
                        format!("goes to {name}, which runs shell commands and scripts")
                    })
            });
            if let Some(why) = runs {
                return Err(refusal(
                    tool,
                    "RUNS_CODE",
                    &format!(
                        "This {} {why}. Outside YOLO Agent-Desk runs only what the apps' own \
                         dictionaries and URLs offer, since a page or file a task read could \
                         have written it. Use your shell tool for commands (sandboxed), \
                         hex_browser_tab for a page in the person's browser, or the app's own \
                         commands; or tell the person it needs YOLO.",
                        recipe.intent
                    ),
                ));
            }
        }
        if let Some(category) = recipe.consequence {
            let authorized = self.store.authorized(&self.config.task);
            let names: Vec<&str> = apps.iter().map(|(name, _, _)| name.as_str()).collect();
            autonomy::Gate::new(tool, mode, authorized)
                .check(category, &format!("this {}", recipe.intent))
                .map_err(|refused| {
                    super::consent::needs_confirmation(&refused.in_app(&names.join(", ")))
                })?;
        }
        Ok(())
    }

    /// The installed app `app` names (its name or bundle id, compared as
    /// app names are).
    fn installed_app(&mut self, app: &str) -> Option<super::super::screen::App> {
        let key = crate::assistant::guard::app_key(app);
        self.screen.installed_apps().into_iter().find(|candidate| {
            crate::assistant::guard::app_key(&candidate.name) == key
                || candidate
                    .bundle_id
                    .as_deref()
                    .is_some_and(|id| crate::assistant::guard::app_key(id) == key)
        })
    }

    /// `hex_app_capabilities`: what an installed app offers, read from the
    /// app itself (`actions::capabilities`).
    pub(super) fn app_capabilities(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, Value> {
        let app = text(args, "app").unwrap_or_default();
        if app.trim().is_empty() {
            return Err(refusal(tool, "INVALID_ARGS", "Name the app with app."));
        }
        let Some(found) = self.installed_app(&app) else {
            return Err(refusal(
                tool,
                "APP_NOT_FOUND",
                &format!("No app “{app}” is installed; hex_installed_apps lists them."),
            ));
        };
        if self.blocked(&found.name)
            || found
                .bundle_id
                .as_deref()
                .is_some_and(|id| self.blocked(id))
        {
            return Err(self.blocked_refusal(tool, &found.name));
        }
        Ok(success("app-capabilities", (self.discover)(&found)))
    }

    pub(super) fn read_window(
        &mut self,
        tool: &str,
        args: &mut Map<String, Value>,
    ) -> Result<Value, Value> {
        let target = match (text(args, "window_id"), text(args, "app")) {
            (Some(id), _) => Target::Window(id),
            (None, Some(app)) => Target::App(app),
            (None, None) => {
                return Err(refusal(
                    tool,
                    "INVALID_ARGS",
                    "Name the window with window_id (or the app with app).",
                ));
            }
        };
        let kind = super::super::policy::Kind::Observe;
        let found: Vec<Resolved> = self.resolve(tool, kind, target, args)?;
        let held = self.held();
        for window in &found {
            self.check(tool, kind, window, &held)?;
        }
        let Some(id) = found.first().and_then(|window| window.id.clone()) else {
            return Err(refusal(
                tool,
                "WINDOW_NOT_FOUND",
                "That app has no window to read.",
            ));
        };
        let number = crate::assistant::computer::window_number(&id)
            .ok_or_else(|| refusal(tool, "WINDOW_NOT_FOUND", "That isn't a window id (w-1234)."))?;
        let size = self
            .list(Look::Check)
            .ok()
            .and_then(|windows| windows.into_iter().find(|window| window.id == id))
            .and_then(|window| window.bounds)
            .or_else(|| {
                self.screen
                    .off_screen_windows()
                    .into_iter()
                    .find(|window| window.id == id)
                    .and_then(|window| window.bounds)
            })
            .map(|(_, _, width, height)| (width, height));
        let reading = self
            .screen
            .read_window(number, size)
            .map_err(|why| failure(&format!("Agent-Desk couldn't read {id}: {why}")))?;
        let total = reading.lines.len();
        let lines: Vec<&ocr::Line> = reading.lines.iter().take(MAX_LINES).collect();
        Ok(success(
            "read-window",
            json!({
                "window": id,
                "text": ocr::text(&reading.lines[..total.min(MAX_LINES)]),
                "lines": lines,
                "more_lines": total.saturating_sub(MAX_LINES),
                "cached": reading.cached,
                "note": "Coordinates are points from the window's top-left corner. Read only: \
                         act with refs from a snapshot where the app has them, else (with real \
                         input allowed) at these points plus the window's position.",
            }),
        ))
    }
}

/// `hex_app_action`'s schema.
pub(super) fn app_action_tool() -> Value {
    json!({
        "name": APP_ACTION_TOOL,
        "description": "Agent-Desk: any installed app's action by its fastest way, without clicking, \
            without its window, and without bringing anything in front. intent script \
            (params.script): AppleScript run inside `tell application id …` of the app, to read \
            or change its data on any desktop (find what its dictionary has with \
            hex_app_capabilities; search with `whose`). intent open_url (params.url): a URL the \
            app takes (its own scheme, a file, a page), opened in the background; a window it \
            opens is yours, parked. A script that sends, deletes, pays, shares, or publishes \
            follows the person's autonomy mode. Outside YOLO it runs no shell commands, other \
            scripts, raw Apple events, or programs, and no app that runs shell scripts.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "app": {"type": "string", "description": "The app's name or bundle id"},
                "intent": {"type": "string", "enum": ["script", "open_url"]},
                "params": {
                    "type": "object",
                    "description": "{\"script\": \"…\"} or {\"url\": \"…\"}",
                    "additionalProperties": {"type": "string"}
                }
            },
            "required": ["app", "intent"],
        },
    })
}

/// `hex_app_capabilities`' schema.
pub(super) fn capabilities_tool() -> Value {
    json!({
        "name": APP_CAPABILITIES_TOOL,
        "description": "Agent-Desk: what any installed app offers, read from the app itself in \
            milliseconds: its scripting dictionary (classes with their properties and \
            elements, commands with parameters), the URL schemes and documents it declares, \
            what its windows are made of (native, Electron, Catalyst), and the ways to reach \
            it, fastest first. Call it before working in an app you haven't scripted in this \
            task. Read-only.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "app": {"type": "string", "description": "The app's name or bundle id"}
            },
            "required": ["app"],
        },
    })
}

/// `hex_read_window`'s schema.
pub(super) fn read_window_tool() -> Value {
    json!({
        "name": READ_WINDOW_TOOL,
        "description": "Agent-Desk: a window's text read from its picture (on-device text recognition), \
            as lines with where they sit in the window. For apps without an Accessibility tree \
            (Citrix, remote desktops, canvases) where a snapshot shows nothing; works on parked \
            and covered windows. Read-only; about a second.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "window_id": {"type": "string", "description": "The window's id (w-1234)"},
                "app": {"type": "string", "description": "Or the app, when it has one window"}
            },
        },
    })
}
