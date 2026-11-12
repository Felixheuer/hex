//! A task's browsers: its main one and up to seven more it names ("a", "b",
//! "suppliers-2"), each an agent-browser session of its own in Hex's
//! namespace (`hex-<task>`, `hex-<task>_<name>`), so one job can read many
//! pages at once.
//!
//! Each starts as one of two kinds and stays that way:
//! - **clean** (the default) for reading and research: a light Lightpanda
//!   browser when it's installed (about a tenth of Chrome's memory), else a
//!   clean Chrome;
//! - **signed in** (`signedIn: true` on its first call) for web apps such as
//!   Gmail or Google Sheets: real Chrome, starting from the sites the person
//!   signed in to for Hex (never saved back).

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::SIGN_IN;

/// Browsers one task may have open at once, its main one included.
pub const MAX_SESSIONS: usize = 12;

/// Arguments no task passes through to agent-browser: they would show a
/// window, reach another task's browsers, keep state, or pass raw flags.
/// `session` and `signedIn` are Hex's own and read here.
pub const WITHHELD_ARGUMENTS: &[&str] = &[
    "namespace",
    "all",
    "headed",
    "extraArgs",
    "restore",
    "restoreSave",
    "restoreCheckUrl",
    "restoreCheckText",
    "restoreCheckFn",
    "capture",
    "caCert",
    "useSystemCa",
    "clearCaCert",
    "idleTimeout",
];

/// The tool that lists a task's browsers, answered by Hex.
pub const LIST_TOOL: &str = "hex_browser_sessions";

/// How a browser started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Clean,
    SignedIn,
}

/// The browser engine behind a clean session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
    Chrome,
    Lightpanda,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Lightpanda => "lightpanda",
        }
    }
}

/// The agent-browser session for `name` of `task` ("" is its main one).
pub fn full_name(task: &str, name: &str) -> String {
    let main = super::session(task);
    if name.is_empty() {
        main
    } else {
        format!("{main}_{name}")
    }
}

/// Whether `session` is one of `task`'s.
pub fn belongs(task: &str, session: &str) -> bool {
    let main = super::session(task);
    session == main
        || session
            .strip_prefix(&main)
            .is_some_and(|rest| rest.starts_with('_'))
}

/// A task's browsers as its MCP server has them.
#[derive(Clone, Debug)]
pub struct Sessions {
    task: String,
    clean: Engine,
    open: BTreeMap<String, Kind>,
}

impl Sessions {
    pub fn new(task: &str, clean: Engine) -> Self {
        Self {
            task: task.to_owned(),
            clean,
            open: BTreeMap::new(),
        }
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    /// A tool call's arguments pinned to one of this task's browsers;
    /// `Err` is the tool error to answer with instead of calling.
    pub fn pin(&mut self, tool: &str, arguments: &mut Map<String, Value>) -> Result<(), String> {
        for name in WITHHELD_ARGUMENTS {
            arguments.remove(*name);
        }
        let name = parse_name(arguments.remove("session"))?;
        let signed_in = arguments
            .remove("signedIn")
            .and_then(|value| value.as_bool());
        arguments.insert("session".into(), full_name(&self.task, &name).into());
        if tool == "agent_browser_close" {
            self.open.remove(&name);
            return Ok(());
        }
        let wanted = signed_in.map(|on| if on { Kind::SignedIn } else { Kind::Clean });
        let kind = match (self.open.get(&name), wanted) {
            (Some(kind), Some(wanted)) if *kind != wanted => {
                return Err(format!(
                    "Browser “{}” started {}; use another session name for a {} one",
                    shown(&name),
                    describe(*kind),
                    describe(wanted)
                ));
            }
            (Some(kind), _) => *kind,
            (None, wanted) => {
                if self.open.len() >= MAX_SESSIONS {
                    return Err(format!(
                        "This task already has {MAX_SESSIONS} browsers ({}). Close one \
                         (agent_browser_close with its session) before opening another",
                        self.open
                            .keys()
                            .map(|name| shown(name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let kind = wanted.unwrap_or(Kind::Clean);
                self.open.insert(name.clone(), kind);
                kind
            }
        };
        let engine = match kind {
            Kind::SignedIn => {
                arguments.insert("restore".into(), SIGN_IN.into());
                arguments.insert("restoreSave".into(), "never".into());
                Engine::Chrome
            }
            Kind::Clean => self.clean,
        };
        arguments.insert("extraArgs".into(), json!(["--engine", engine.name()]));
        Ok(())
    }

    /// The task's browsers for [`LIST_TOOL`]; `running` are the sessions
    /// agent-browser has open.
    pub fn list(&self, running: &[String]) -> Value {
        let browsers: Vec<Value> = self
            .open
            .iter()
            .map(|(name, kind)| {
                let engine = match kind {
                    Kind::SignedIn => Engine::Chrome,
                    Kind::Clean => self.clean,
                };
                json!({
                    "session": shown(name),
                    "signedIn": *kind == Kind::SignedIn,
                    "engine": engine.name(),
                    "running": running.contains(&full_name(&self.task, name)),
                })
            })
            .collect();
        json!({ "browsers": browsers, "max": MAX_SESSIONS })
    }
}

fn shown(name: &str) -> &str {
    if name.is_empty() { "main" } else { name }
}

fn describe(kind: Kind) -> &'static str {
    match kind {
        Kind::Clean => "clean",
        Kind::SignedIn => "signed in",
    }
}

/// A `session` argument: absent, "", or "main" is the main browser; else 1
/// to 16 letters, digits, `-`, or `_`, lowercased.
fn parse_name(value: Option<Value>) -> Result<String, String> {
    let text = match value {
        None | Some(Value::Null) => return Ok(String::new()),
        Some(Value::String(text)) => text.trim().to_lowercase(),
        Some(other) => return Err(format!("session must be a short name, not {other}")),
    };
    if text.is_empty() || text == "main" {
        return Ok(String::new());
    }
    let valid = text.chars().count() <= 16
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(text)
    } else {
        Err(format!(
            "session “{text}” isn't a valid name: use up to 16 letters, digits, - or _"
        ))
    }
}

/// The `session` and `signedIn` arguments Hex adds to every tool's schema,
/// and the list tool.
pub fn describe_tools(result: &mut Value) {
    let last_page = result.get("nextCursor").is_none_or(Value::is_null);
    let Some(tools) = result.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for tool in tools.iter_mut() {
        let opens = matches!(
            tool.get("name").and_then(Value::as_str),
            Some("agent_browser_open" | "agent_browser_tab_new")
        );
        let Some(schema) = tool.get_mut("inputSchema") else {
            continue;
        };
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            for name in WITHHELD_ARGUMENTS {
                properties.remove(*name);
            }
            properties.insert(
                "session".into(),
                json!({"type": "string", "description": format!(
                    "Which of this task's browsers: omit for the main one, or a short name \
                     (a, b, … or up to 16 letters, digits, - and _) for another one that runs \
                     beside it. Up to {MAX_SESSIONS} per task.")}),
            );
            if opens {
                properties.insert(
                    "signedIn".into(),
                    json!({"type": "boolean", "description":
                        "On a browser's first call only: true starts it in real Chrome with \
                         the sites the person signed in to for Hex (for web apps such as \
                         Gmail or Google Sheets). Default false: a clean, light browser for \
                         reading and research."}),
                );
            }
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| {
                name.as_str()
                    .is_none_or(|name| !WITHHELD_ARGUMENTS.contains(&name) && name != "session")
            });
        }
    }
    if last_page {
        tools.push(json!({
            "name": LIST_TOOL,
            "title": "This task's browsers",
            "description": "List this task's browsers: each one's session name, whether it \
                started signed in, its engine, and whether it is running. Cheap; touches no page.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn the_main_browser_is_clean_and_light_by_default() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda);
        let mut call = args(json!({"url": "https://example.com", "headed": true,
            "extraArgs": ["--headed"], "namespace": "x", "restore": true}));
        sessions.pin("agent_browser_open", &mut call).unwrap();
        assert_eq!(
            Value::Object(call),
            json!({"url": "https://example.com", "session": "hex-t1",
                "extraArgs": ["--engine", "lightpanda"]})
        );
    }

    #[test]
    fn a_named_signed_in_browser_runs_chrome_from_the_sign_ins() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda);
        let mut call =
            args(json!({"session": "Mail", "signedIn": true, "url": "https://mail.google.com"}));
        sessions.pin("agent_browser_open", &mut call).unwrap();
        assert_eq!(call["session"], "hex-t1_mail");
        assert_eq!(call["restore"], SIGN_IN);
        assert_eq!(call["restoreSave"], "never");
        assert_eq!(call["extraArgs"], json!(["--engine", "chrome"]));
        // Later calls keep its kind without saying it again.
        let mut later = args(json!({"session": "mail"}));
        sessions.pin("agent_browser_snapshot", &mut later).unwrap();
        assert_eq!(later["restore"], SIGN_IN);
        // It can't turn into a clean one.
        let mut clean = args(json!({"session": "mail", "signedIn": false}));
        assert!(sessions.pin("agent_browser_open", &mut clean).is_err());
    }

    #[test]
    fn a_task_has_at_most_twelve_browsers_and_closing_frees_one() {
        let mut sessions = Sessions::new("t1", Engine::Chrome);
        for name in [
            "main", "a", "b", "c", "d", "e", "f", "g", "i", "j", "k", "l",
        ] {
            sessions
                .pin("agent_browser_open", &mut args(json!({"session": name})))
                .unwrap();
        }
        let refused = sessions.pin("agent_browser_open", &mut args(json!({"session": "h"})));
        assert!(refused.unwrap_err().contains("already has 12 browsers"));
        let mut close = args(json!({"session": "c", "all": true}));
        sessions.pin("agent_browser_close", &mut close).unwrap();
        assert_eq!(Value::Object(close), json!({"session": "hex-t1_c"}));
        sessions
            .pin("agent_browser_open", &mut args(json!({"session": "h"})))
            .unwrap();
    }

    #[test]
    fn session_names_stay_inside_the_task() {
        let mut sessions = Sessions::new("t1", Engine::Chrome);
        for bad in [json!("../x"), json!("a b"), json!("x".repeat(17)), json!(3)] {
            let refused = sessions.pin("agent_browser_open", &mut args(json!({"session": bad})));
            assert!(refused.is_err(), "{bad}");
        }
        assert!(belongs("t1", "hex-t1"));
        assert!(belongs("t1", "hex-t1_a"));
        assert!(!belongs("t1", "hex-t10"));
        assert!(!belongs("t1", "hex-t1-x"));
    }

    #[test]
    fn the_list_says_what_each_browser_is() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda);
        sessions
            .pin("agent_browser_open", &mut args(json!({})))
            .unwrap();
        sessions
            .pin(
                "agent_browser_open",
                &mut args(json!({"session": "a", "signedIn": true})),
            )
            .unwrap();
        assert_eq!(
            sessions.list(&["hex-t1".into()]),
            json!({"browsers": [
                {"session": "main", "signedIn": false, "engine": "lightpanda", "running": true},
                {"session": "a", "signedIn": true, "engine": "chrome", "running": false},
            ], "max": 12})
        );
    }

    #[test]
    fn every_tool_offers_session_and_open_offers_signed_in() {
        let mut result = json!({"tools": [
            {"name": "agent_browser_open", "inputSchema": {"type": "object", "required": ["session"],
                "properties": {"url": {"type": "string"}, "session": {"type": "string"},
                    "headed": {"type": "boolean"}, "restore": {}}}},
            {"name": "agent_browser_close", "inputSchema": {"type": "object",
                "properties": {"all": {"type": "boolean"}}}},
        ]});
        describe_tools(&mut result);
        let tools = result["tools"].as_array().unwrap();
        let open = &tools[0]["inputSchema"];
        let mut keys: Vec<_> = open["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(keys, ["session", "signedIn", "url"]);
        assert_eq!(open["required"], json!([]));
        assert!(tools[1]["inputSchema"]["properties"].get("all").is_none());
        assert!(
            tools[1]["inputSchema"]["properties"]
                .get("signedIn")
                .is_none()
        );
        assert_eq!(tools[2]["name"], LIST_TOOL);
    }
}
