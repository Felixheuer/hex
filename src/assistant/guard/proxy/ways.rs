//! What worked in which app, from what the guard sees (`procedures`): each
//! action it forwarded and whether the app took it, as a way of doing
//! things in that app (a menu path, an app action, a skill, a control by
//! its role and name, keys, or a spot of the picture). Refusals of the
//! guard's own are no method's failure and are left out; typed text and
//! values never are kept, only what was acted on.

use serde_json::{Map, Value};

use super::super::child::Desktop;
use super::{Guard, Resolved, text};
use crate::assistant::procedures::{self, Method, Outcome};

/// Roles worth remembering an action on: controls, not content.
const CONTROLS: &[&str] = &[
    "button",
    "menuitem",
    "menu",
    "checkbox",
    "radio",
    "tab",
    "popup",
    "combobox",
    "link",
    "toolbar",
    "segment",
    "switch",
    "textfield",
    "searchfield",
    "textarea",
    "slider",
];

/// Whether the guard refused `result` itself (not the app's failure).
fn refused_by_guard(result: &Value) -> bool {
    result["structuredContent"]["error"]["details"]["kind"] == "hex_guard"
        || result["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("Agent-Desk:"))
}

/// Why `result` failed, in a line.
fn why(result: &Value) -> Option<String> {
    let error = &result["structuredContent"]["error"]["message"];
    error
        .as_str()
        .or_else(|| result["content"][0]["text"].as_str())
        .map(|text| text.lines().next().unwrap_or_default().to_owned())
}

/// The way an action of `tool` with `args` is remembered: its method and
/// what exactly; `None` for what isn't a way of doing something (a look).
pub(super) fn way(
    tool: &str,
    args: &Map<String, Value>,
    element: Option<(&str, &str)>,
) -> Option<(Method, String)> {
    let action = tool
        .trim_start_matches("desktop_")
        .trim_start_matches("hex_");
    match action {
        "menu" => {
            let path = match args.get("path") {
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" > "),
                Some(Value::String(path)) => path.clone(),
                _ => return None,
            };
            Some((Method::Menu, path))
        }
        "press" | "hotkey" => text(args, "combo")
            .or_else(|| text(args, "key"))
            .or_else(|| text(args, "keys"))
            .map(|combo| (Method::Keys, combo)),
        "app_action" => {
            let intent = text(args, "intent").unwrap_or_default();
            let params = args.get("params").and_then(Value::as_object);
            let detail = match intent.as_str() {
                "open_url" => params
                    .and_then(|params| params.get("url"))
                    .and_then(Value::as_str)
                    .map(|url| url.split(['?', '#']).next().unwrap_or(url).to_owned())
                    .unwrap_or_else(|| "open_url".into()),
                // A script's text may hold the person's words: only its kind.
                "script" => "AppleScript (hex_app_action script)".into(),
                other => format!("hex_app_action {other}"),
            };
            let method = if intent == "script" {
                Method::Script
            } else {
                Method::Url
            };
            Some((method, detail))
        }
        "run_script" => Some((Method::Script, "desktop_run_script".into())),
        "skill_replay" => text(args, "intent").map(|intent| (Method::Skill, intent)),
        "window_input" => Some((
            Method::Visual,
            format!(
                "{} at a spot of the picture",
                text(args, "action").unwrap_or_else(|| "input".into())
            ),
        )),
        "click" | "double_click" | "right_click" | "type" | "set_value" | "select" | "toggle"
        | "check" | "uncheck" | "expand" | "collapse" | "scroll" | "drag" => {
            if args.contains_key("x") || args.contains_key("y") {
                return Some((Method::Visual, format!("{action} by position")));
            }
            let (role, name) = element?;
            let role_key = role.to_lowercase().trim_start_matches("ax").to_owned();
            if name.trim().is_empty() || !CONTROLS.iter().any(|control| role_key.contains(control))
            {
                return None;
            }
            Some((Method::Ax, format!("{action} {role} “{}”", name.trim())))
        }
        _ => None,
    }
}

impl<D: Desktop> Guard<D> {
    /// Notes how an action of `tool` went in its app (`procedures`).
    pub(super) fn note_way(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
        resolved: &[Resolved],
        result: &Value,
    ) {
        if self.reader || refused_by_guard(result) {
            return;
        }
        let Some(app) = resolved
            .first()
            .map(|window| window.app.clone())
            .or_else(|| text(args, "app"))
        else {
            return;
        };
        let element = text(args, "ref_id")
            .and_then(|reference| self.learned.elements.lock().get(&reference).cloned());
        if element.as_ref().is_some_and(|element| element.secure) {
            return;
        }
        let Some((method, detail)) = way(
            tool,
            args,
            element
                .as_ref()
                .map(|element| (element.role.as_str(), element.label.as_str()))
                .filter(|(_, label)| {
                    element.as_ref().is_some_and(|e| e.label != e.value) && !label.is_empty()
                }),
        ) else {
            return;
        };
        let ok = result["isError"] != true;
        let version = self.app_version(&app);
        procedures::record(&Outcome {
            app,
            version,
            method,
            detail,
            ok,
            why: (!ok).then(|| why(result)).flatten(),
            project: None,
        });
    }

    /// The installed version of `app`, looked up once per guard.
    fn app_version(&mut self, app: &str) -> Option<String> {
        let key = crate::assistant::guard::app_key(app);
        if let Some(known) = self.versions.get(&key) {
            return known.clone();
        }
        let version = if cfg!(test) {
            None
        } else {
            self.screen
                .installed_apps()
                .into_iter()
                .find(|candidate| crate::assistant::guard::app_key(&candidate.name) == key)
                .and_then(|found| procedures::bundle_version(&found.path))
        };
        self.versions.insert(key, version.clone());
        version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn actions_become_ways_without_what_was_typed() {
        assert_eq!(
            way(
                "desktop_menu",
                &args(json!({"path": ["File", "Export To", "PDF…"]})),
                None
            ),
            Some((Method::Menu, "File > Export To > PDF…".into()))
        );
        assert_eq!(
            way(
                "desktop_click",
                &args(json!({"ref_id": "s1:e4"})),
                Some(("AXButton", "Export"))
            ),
            Some((Method::Ax, "click AXButton “Export”".into()))
        );
        // Typed text is never part of it; only the field.
        let typed = way(
            "desktop_type",
            &args(json!({"ref_id": "s1:e5", "text": "Hallo Max, anbei die Rechnung"})),
            Some(("AXTextField", "Subject")),
        )
        .unwrap();
        assert_eq!(typed.1, "type AXTextField “Subject”");
        // Content (a cell, a row) isn't a way of doing something.
        assert_eq!(
            way(
                "desktop_click",
                &args(json!({})),
                Some(("AXCell", "Max Mustermann"))
            ),
            None
        );
        assert_eq!(
            way("desktop_click", &args(json!({"x": 10, "y": 20})), None),
            Some((Method::Visual, "click by position".into()))
        );
        assert_eq!(
            way(
                "hex_app_action",
                &args(
                    json!({"intent": "script", "params": {"script": "tell app \"Mail\" to send"}})
                ),
                None
            ),
            Some((Method::Script, "AppleScript (hex_app_action script)".into()))
        );
        assert_eq!(
            way(
                "hex_app_action",
                &args(
                    json!({"intent": "open_url", "params": {"url": "https://x.example/a?token=1"}})
                ),
                None
            ),
            Some((Method::Url, "https://x.example/a".into()))
        );
        assert_eq!(way("desktop_snapshot", &args(json!({})), None), None);
        assert!(refused_by_guard(&super::super::refusal(
            "desktop_click",
            "POLICY_DENIED",
            "no"
        )));
    }
}
