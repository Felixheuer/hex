//! agent-desktop calls as a person reads them ("Clicking “Reply” in Mail"),
//! and the windows a task works in, learned from the calls' arguments and
//! results. Every backend reports its desktop tool calls through here.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::Report;
use crate::assistant::types::TaskWindow;

/// The agent-desktop command a tool name stands for (`click` for
/// `mcp__agent-desktop__desktop_click`, `agent-desktop_desktop_click`, or
/// `desktop_click`), if it is one.
pub fn command(tool: &str) -> Option<&str> {
    let tool = tool
        .strip_prefix("mcp__agent-desktop__")
        .or_else(|| tool.strip_prefix("agent-desktop_"))
        .or_else(|| tool.strip_prefix("agent_desktop_"))
        .unwrap_or(tool);
    tool.strip_prefix("desktop_")
}

/// A hidden-browser call as a step: "Opening example.com in the browser".
fn browser_step(tool: &str, args: &Value) -> Option<Report> {
    let name = tool
        .strip_prefix("mcp__browser__")
        .or_else(|| tool.strip_prefix("browser_"))?;
    let host = text(args, "url").map(|url| {
        url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| url.to_owned())
    });
    let (doing, done) = match (name, host) {
        (_, Some(host)) => (
            format!("Opening {host} in the browser"),
            format!("Opened {host} in the browser"),
        ),
        (name, None) if name.contains("snapshot") || name.contains("screenshot") => (
            "Reading the page".to_owned(),
            "Read the page".to_owned(),
        ),
        _ => ("Working in the browser".to_owned(), String::new()),
    };
    Some(Report::Step { doing, done })
}

/// What one task has seen of the desktop so far.
#[derive(Debug, Default)]
pub struct Describer {
    /// A ref's element: its role and name, from the snapshot that made it.
    elements: HashMap<String, (String, String)>,
    /// The app each snapshot read.
    snapshot_apps: HashMap<String, String>,
    /// Windows already reported.
    windows: HashSet<(String, Option<String>)>,
}

impl Describer {
    /// The reports for a call of `tool` with `args`, before its result.
    pub fn call(&mut self, tool: &str, args: &Value) -> Vec<Report> {
        let Some(command) = command(tool) else {
            return browser_step(tool, args).into_iter().collect();
        };
        let mut reports = Vec::new();
        if let Some(window) = self.window_of(command, args) {
            reports.extend(self.window(window));
        }
        if let Some((doing, done)) = self.describe(command, args) {
            reports.push(Report::Step { doing, done });
        }
        reports
    }

    /// Learns from a call's `output` (agent-desktop's JSON envelope, or the
    /// MCP text that holds it): the refs a snapshot named, the window an app
    /// opened in.
    pub fn result(&mut self, tool: &str, output: &str) -> Vec<Report> {
        let Some(command) = command(tool) else {
            return Vec::new();
        };
        let Some(envelope) = envelope(output) else {
            return Vec::new();
        };
        let data = envelope.get("data").unwrap_or(&Value::Null);
        if let (Some(snapshot), Some(app)) = (
            data.get("snapshot_id").and_then(Value::as_str),
            data.get("app").and_then(Value::as_str),
        ) {
            self.snapshot_apps
                .insert(snapshot.to_owned(), app.to_owned());
        }
        collect_elements(data, &mut self.elements);
        if command == "launch"
            && let (Some(app), Some(id)) = (
                data.get("app").and_then(Value::as_str),
                data.pointer("/window/id").and_then(Value::as_str),
            )
        {
            return self.window(TaskWindow {
                app: app.to_owned(),
                window_id: Some(id.to_owned()),
            });
        }
        Vec::new()
    }

    fn window(&mut self, window: TaskWindow) -> Vec<Report> {
        let key = (window.app.clone(), window.window_id.clone());
        let known = self.windows.contains(&key)
            || (window.window_id.is_none()
                && self.windows.iter().any(|(app, _)| *app == window.app));
        if known {
            return Vec::new();
        }
        self.windows.insert(key);
        vec![Report::Window(window)]
    }

    /// The window a call works in: its `app` (and `window_id`), or the app
    /// of the snapshot its ref came from.
    fn window_of(&self, command: &str, args: &Value) -> Option<TaskWindow> {
        if matches!(
            command,
            "list_apps" | "list_windows" | "list_displays" | "close_app" | "status" | "version"
        ) || command.starts_with("skills")
            || command.starts_with("clipboard")
            || command.contains("notification")
        {
            return None;
        }
        let app = text(args, "app")
            .map(str::to_owned)
            .or_else(|| self.ref_app(args))?;
        Some(TaskWindow {
            app,
            window_id: text(args, "window_id").map(str::to_owned),
        })
    }

    fn ref_app(&self, args: &Value) -> Option<String> {
        let reference = ["ref_id", "root", "element", "from", "to"]
            .iter()
            .find_map(|key| text(args, key))?;
        self.snapshot_apps.get(snapshot_of(reference)?).cloned()
    }

    /// "Clicking “Reply” in Mail" and "Clicked “Reply” in Mail".
    fn describe(&self, command: &str, args: &Value) -> Option<(String, String)> {
        let target = self.target(args);
        let app = text(args, "app")
            .map(str::to_owned)
            .or_else(|| self.ref_app(args));
        let in_app = app
            .as_deref()
            .map(|app| format!(" in {app}"))
            .unwrap_or_default();
        let both = |doing: String, done: String| Some((doing, done));
        match command {
            "snapshot" => {
                let what = app.unwrap_or_else(|| "the screen".into());
                both(format!("Looking at {what}"), format!("Looked at {what}"))
            }
            "find" => {
                let what = ["name", "text", "value", "role"]
                    .iter()
                    .find_map(|key| text(args, key))
                    .map(|what| format!("“{what}”"))
                    .unwrap_or_else(|| "something".into());
                both(
                    format!("Looking for {what}{in_app}"),
                    format!("Looked for {what}{in_app}"),
                )
            }
            "screenshot" => both(
                format!("Taking a screenshot{in_app}"),
                format!("Took a screenshot{in_app}"),
            ),
            "get" | "is" => both(format!("Reading {target}"), format!("Read {target}")),
            "click" | "double_click" | "triple_click" => {
                both(format!("Clicking {target}"), format!("Clicked {target}"))
            }
            "right_click" => both(
                format!("Opening the menu of {target}"),
                format!("Opened the menu of {target}"),
            ),
            "type" => {
                let count = text(args, "text").map_or(0, |text| text.chars().count());
                let s = if count == 1 { "" } else { "s" };
                both(
                    format!("Typing {count} character{s} into {target}"),
                    format!("Typed {count} character{s} into {target}"),
                )
            }
            "set_value" => both(
                format!("Filling in {target}"),
                format!("Filled in {target}"),
            ),
            "clear" => both(format!("Clearing {target}"), format!("Cleared {target}")),
            "focus" => both(format!("Focusing {target}"), format!("Focused {target}")),
            "select" => {
                let value = text(args, "value").unwrap_or("an option");
                both(
                    format!("Choosing “{value}” in {target}"),
                    format!("Chose “{value}” in {target}"),
                )
            }
            "toggle" => both(format!("Switching {target}"), format!("Switched {target}")),
            "check" => both(format!("Checking {target}"), format!("Checked {target}")),
            "uncheck" => both(
                format!("Unchecking {target}"),
                format!("Unchecked {target}"),
            ),
            "expand" => both(format!("Expanding {target}"), format!("Expanded {target}")),
            "collapse" => both(
                format!("Collapsing {target}"),
                format!("Collapsed {target}"),
            ),
            "scroll" | "scroll_to" | "mouse_wheel" => {
                both(format!("Scrolling {target}"), format!("Scrolled {target}"))
            }
            "press" => {
                let combo = text(args, "combo").unwrap_or("a key");
                both(
                    format!("Pressing {combo}{in_app}"),
                    format!("Pressed {combo}{in_app}"),
                )
            }
            "hover" | "mouse_move" => both("Moving the pointer".into(), "Moved the pointer".into()),
            "drag" => both("Dragging".into(), "Dragged".into()),
            "mouse_click" => both(
                "Clicking on the screen".into(),
                "Clicked on the screen".into(),
            ),
            "launch" => {
                let app = text(args, "app").unwrap_or("an app");
                both(format!("Opening {app}"), format!("Opened {app}"))
            }
            "close_app" => {
                let app = text(args, "app").unwrap_or("an app");
                both(format!("Quitting {app}"), format!("Quit {app}"))
            }
            "list_apps" => both("Checking open apps".into(), "Checked open apps".into()),
            "list_windows" => {
                let whose = text(args, "app")
                    .map(|app| format!("{app}’s windows"))
                    .unwrap_or_else(|| "open windows".into());
                both(format!("Checking {whose}"), format!("Checked {whose}"))
            }
            "list_displays" => both("Checking displays".into(), "Checked displays".into()),
            "focus_window" => {
                let what = app.unwrap_or_else(|| "a window".into());
                both(
                    format!("Bringing {what} to the front"),
                    format!("Brought {what} to the front"),
                )
            }
            "resize_window" | "move_window" => {
                let what = app.unwrap_or_else(|| "a window".into());
                both(format!("Arranging {what}"), format!("Arranged {what}"))
            }
            "minimize" | "maximize" | "restore" => {
                let what = app.unwrap_or_else(|| "a window".into());
                let (doing, done) = match command {
                    "minimize" => ("Minimizing", "Minimized"),
                    "maximize" => ("Maximizing", "Maximized"),
                    _ => ("Restoring", "Restored"),
                };
                both(format!("{doing} {what}"), format!("{done} {what}"))
            }
            "list_surfaces" | "open_system_surface" => {
                both("Looking at menus".into(), "Looked at menus".into())
            }
            "list_notifications" => {
                both("Reading notifications".into(), "Read notifications".into())
            }
            "dismiss_notification" | "dismiss_all_notifications" => both(
                "Dismissing notifications".into(),
                "Dismissed notifications".into(),
            ),
            "notification_action" => {
                let action = text(args, "action").unwrap_or("an action");
                both(
                    format!("Choosing “{action}” in a notification"),
                    format!("Chose “{action}” in a notification"),
                )
            }
            "clipboard_get" => both("Reading the clipboard".into(), "Read the clipboard".into()),
            "clipboard_set" => both(
                "Copying to the clipboard".into(),
                "Copied to the clipboard".into(),
            ),
            "clipboard_clear" => both(
                "Clearing the clipboard".into(),
                "Cleared the clipboard".into(),
            ),
            "wait" => Some(("Waiting".into(), String::new())),
            "batch" => {
                let count = text(args, "commands_json")
                    .and_then(|json| serde_json::from_str::<Vec<Value>>(json).ok())
                    .map_or(0, |list| list.len());
                both(format!("Doing {count} steps"), format!("Did {count} steps"))
            }
            _ => None,
        }
    }

    /// The element a call's ref names, as a person reads it.
    fn target(&self, args: &Value) -> String {
        let Some(reference) = text(args, "ref_id") else {
            return "it".into();
        };
        let in_app = snapshot_of(reference)
            .and_then(|snapshot| self.snapshot_apps.get(snapshot))
            .map(|app| format!(" in {app}"))
            .unwrap_or_default();
        match self.elements.get(reference) {
            Some((_, name)) if !name.is_empty() => format!("“{}”{in_app}", short(name)),
            Some((role, _)) if !role.is_empty() => format!("a {role}{in_app}"),
            _ => format!("an element{in_app}"),
        }
    }
}

/// agent-desktop's JSON envelope inside `output`: the text itself, MCP
/// content (`[{"type":"text","text":…}]` or `{"content":[…]}`), or a JSON
/// string holding either.
fn envelope(output: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(output.trim()).ok()?;
    unwrap_envelope(value)
}

fn unwrap_envelope(value: Value) -> Option<Value> {
    match value {
        Value::Object(ref map) if map.contains_key("data") || map.contains_key("ok") => Some(value),
        Value::Object(mut map) => unwrap_envelope(map.remove("content")?),
        Value::Array(items) => items.into_iter().find_map(|item| match item {
            Value::Object(mut map) => unwrap_envelope(map.remove("text")?),
            other => unwrap_envelope(other),
        }),
        Value::String(text) => envelope(&text),
        _ => None,
    }
}

fn collect_elements(value: &Value, elements: &mut HashMap<String, (String, String)>) {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("ref_id").and_then(Value::as_str) {
                let role = map.get("role").and_then(Value::as_str).unwrap_or_default();
                let name = ["name", "description", "title", "value"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_default();
                elements.insert(reference.to_owned(), (role.to_owned(), name.to_owned()));
            }
            for child in map.values() {
                collect_elements(child, elements);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_elements(item, elements);
            }
        }
        _ => {}
    }
}

/// `s8f3k2p9` from `@s8f3k2p9:e5`.
fn snapshot_of(reference: &str) -> Option<&str> {
    reference.strip_prefix('@')?.split(':').next()
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn short(name: &str) -> String {
    crate::assistant::store::shorten(name, 40)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn steps(reports: &[Report]) -> Vec<&str> {
        reports
            .iter()
            .filter_map(|report| match report {
                Report::Step { doing, .. } => Some(doing.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_click_names_the_element_its_snapshot_found() {
        let mut describer = Describer::default();
        let first = describer.call(
            "mcp__agent-desktop__desktop_snapshot",
            &json!({"app": "Mail", "interactive_only": true}),
        );
        assert_eq!(
            first,
            [
                Report::Window(TaskWindow {
                    app: "Mail".into(),
                    window_id: None
                }),
                Report::Step {
                    doing: "Looking at Mail".into(),
                    done: "Looked at Mail".into()
                },
            ]
        );
        let snapshot = json!({"ok": true, "command": "snapshot", "data": {"app": "Mail", "snapshot_id": "sabc",
            "tree": {"role": "window", "children": [{"ref_id": "@sabc:e5", "role": "button", "name": "Reply"},
                {"ref_id": "@sabc:e6", "role": "textfield"}]}}});
        let mcp = json!([{"type": "text", "text": snapshot.to_string()}]).to_string();
        assert!(
            describer
                .result("mcp__agent-desktop__desktop_snapshot", &mcp)
                .is_empty()
        );

        let click = describer.call("desktop_click", &json!({"ref_id": "@sabc:e5"}));
        assert_eq!(steps(&click), ["Clicking “Reply” in Mail"]);
        let typed = describer.call(
            "agent-desktop_desktop_type",
            &json!({"ref_id": "@sabc:e6", "text": "hello"}),
        );
        assert_eq!(
            typed,
            [Report::Step {
                doing: "Typing 5 characters into a textfield in Mail".into(),
                done: "Typed 5 characters into a textfield in Mail".into()
            }]
        );
    }

    #[test]
    fn a_launch_reports_the_window_it_opened_once() {
        let mut describer = Describer::default();
        let call = describer.call("desktop_launch", &json!({"app": "TextEdit"}));
        assert_eq!(steps(&call), ["Opening TextEdit"]);
        let result = json!({"ok": true, "data": {"app": "TextEdit", "pid": 611, "window": {"id": "w-110407", "title": "Untitled"}}});
        assert_eq!(
            describer.result("desktop_launch", &result.to_string()),
            [Report::Window(TaskWindow {
                app: "TextEdit".into(),
                window_id: Some("w-110407".into())
            })]
        );
        assert!(
            describer
                .result("desktop_launch", &result.to_string())
                .is_empty()
        );
        assert!(
            describer
                .call("desktop_snapshot", &json!({"app": "TextEdit"}))
                .iter()
                .all(|report| !matches!(report, Report::Window(_)))
        );
    }

    #[test]
    fn listings_and_unknown_tools_are_not_windows() {
        let mut describer = Describer::default();
        assert_eq!(
            describer.call("desktop_list_apps", &json!({})),
            [Report::Step {
                doing: "Checking open apps".into(),
                done: "Checked open apps".into()
            }]
        );
        assert!(describer.call("desktop_session_end", &json!({})).is_empty());
        assert!(describer.call("Bash", &json!({"command": "ls"})).is_empty());
        assert_eq!(command("mcp__agent-desktop__desktop_press"), Some("press"));
        assert_eq!(command("WebSearch"), None);
    }
}
