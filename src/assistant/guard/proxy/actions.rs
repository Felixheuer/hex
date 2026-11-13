//! Hex's fast actions for a task (`assistant::actions`): `hex_app_action`
//! (an app recipe: deep link, AppleScript, CLI) and `hex_read_window` (a
//! window's text from its picture), checked like every other action:
//! blocked apps are refused, a consequential recipe needs the person's yes
//! (`autonomy`), and a window the recipe opens is the task's (and, in a
//! quick turn, the desk's), parked out of the person's way.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value, json};

use super::super::child::Desktop;
use super::super::policy::{APP_ACTION_TOOL, READ_WINDOW_TOOL, Target};
use super::apps::success;
use super::{Guard, Launched, Look, Resolved, failure, refusal, text};
use crate::assistant::actions::{ocr, recipes};
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
        let all = recipes::all();
        let (recipe, command) = match recipes::find(all, &app, &intent) {
            Some(recipe) => {
                let command = recipes::build(recipe, &values)
                    .map_err(|why| refusal(tool, "INVALID_ARGS", &why))?;
                (recipe.clone(), command)
            }
            None => {
                // Any installed app: its URLs and its scripting, no recipe
                // needed (the recipes are fast paths, not a limit).
                let key = crate::assistant::guard::app_key(&app);
                let installed = self.screen.installed_apps().into_iter().find(|candidate| {
                    crate::assistant::guard::app_key(&candidate.name) == key
                        || candidate
                            .bundle_id
                            .as_deref()
                            .is_some_and(|id| crate::assistant::guard::app_key(id) == key)
                });
                let (name, bundle_id) = installed
                    .map(|found| (found.name, found.bundle_id))
                    .unwrap_or_else(|| (app.clone(), None));
                match recipes::generic(&name, bundle_id.as_deref(), &intent, &values) {
                    Some(built) => built.map_err(|why| refusal(tool, "INVALID_ARGS", &why))?,
                    None => {
                        let known: Vec<String> = all
                            .iter()
                            .filter(|recipe| crate::assistant::guard::app_key(&recipe.app) == key)
                            .map(|recipe| recipe.intent.clone())
                            .chain(recipes::GENERIC.iter().map(|intent| (*intent).to_owned()))
                            .collect();
                        return Err(refusal(
                            tool,
                            "UNKNOWN_RECIPE",
                            &format!(
                                "{app} has no intent {intent}; it has: {} (open_url and script \
                                 work for any app). Or open it with hex_open_app and act in its \
                                 window.",
                                known.join(", ")
                            ),
                        ));
                    }
                }
            }
        };
        let recipe = &recipe;
        if self.blocked(&recipe.name)
            || recipe
                .bundle_id
                .as_deref()
                .is_some_and(|id| self.blocked(id))
        {
            return Err(self.blocked_refusal(tool, &recipe.name));
        }
        if let Some(category) = recipe.consequence {
            let mode = self.config.autonomy;
            let authorized = self.store.authorized(&self.config.task);
            if mode != Autonomy::Yolo && !autonomy::allowed(mode, category, &authorized) {
                let mut result = refusal(
                    tool,
                    "NEEDS_CONFIRMATION",
                    &format!(
                        "{} {} would {} in {}. The person hasn't allowed that for this task; ask \
                         them and wait for their yes, Hex refuses it until they confirm.",
                        recipe.app,
                        recipe.intent,
                        serde_json::to_value(category)
                            .ok()
                            .and_then(|value| value.as_str().map(str::to_owned))
                            .unwrap_or_default(),
                        recipe.name
                    ),
                );
                let details = &mut result["structuredContent"]["error"]["details"];
                details["category"] = json!(category);
                details["app"] = json!(recipe.name);
                let text = result["structuredContent"].to_string();
                result["content"][0]["text"] = Value::String(text);
                return Err(result);
            }
        }
        // A recipe that shows a window: the app's window comes here first
        // (from another desktop if need be) and becomes the task's.
        let installed = self.screen.installed_apps().into_iter().find(|app| {
            recipe
                .bundle_id
                .as_deref()
                .is_some_and(|id| app.bundle_id.as_deref() == Some(id))
                || super::same_app(&app.name, &recipe.name)
        });
        let running = installed
            .as_ref()
            .is_some_and(|app| self.screen.running_apps().contains(&app.path));
        let shows = recipe.opens_window();
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
        let output = (self.run_recipe)(&command).map_err(|why| {
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
                app: recipe.name.clone(),
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
        self.watch(&recipe.name);
        let mut note = match recipe.consequence {
            None if recipe.intent.contains("draft")
                || recipe.intent == "open_chat"
                || recipe.intent == "compose" =>
            {
                "Done as a draft: nothing was sent.".to_owned()
            }
            _ => "Done.".to_owned(),
        };
        if !windows.is_empty() {
            note.push_str(" Its window is yours: act in it by its id (check with a snapshot).");
        }
        if desk {
            note = format!("{note} {}", super::DESK_NOTE);
        }
        Ok(success(
            "app-action",
            json!({
                "app": recipe.name,
                "intent": recipe.intent,
                "output": output,
                "windows": windows,
                "note": note,
            }),
        ))
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
        let reading = if cfg!(test) {
            Ok(ocr::Reading::default())
        } else {
            ocr::read(number, size)
        }
        .map_err(|why| failure(&format!("Hex couldn't read {id}: {why}")))?;
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

/// `hex_app_action`'s schema, with every recipe listed.
pub(super) fn app_action_tool() -> Value {
    json!({
        "name": APP_ACTION_TOOL,
        "description": format!(
            "Hex: an app's action by its fastest way (deep link, AppleScript, or CLI), without \
             clicking and without bringing anything in front. Prefer it to clicking through an \
             app. Works for every installed app: intent open_url (params.url: any URL the app \
             takes, its own scheme, a file, a page; opened in the background) and intent script \
             (params.script: AppleScript run inside `tell application id …` for any scriptable \
             app). Drafts are never sent; anything that sends, deletes, or pays needs the \
             person's yes. A window it opens is yours (parked). Known fast paths (app \
             intent(params): what it does):\n{}",
            recipes::catalogue(recipes::all())
        ),
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "app": {"type": "string", "description": "The recipe's app, e.g. whatsapp"},
                "intent": {"type": "string", "description": "The recipe's intent, e.g. open_chat"},
                "params": {
                    "type": "object",
                    "description": "The recipe's parameters by name (strings)",
                    "additionalProperties": {"type": "string"}
                }
            },
            "required": ["app", "intent"],
        },
    })
}

/// `hex_read_window`'s schema.
pub(super) fn read_window_tool() -> Value {
    json!({
        "name": READ_WINDOW_TOOL,
        "description": "Hex: a window's text read from its picture (on-device text recognition), \
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
