//! agent-desktop's replies: every command prints one JSON envelope,
//! `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message"}}`.

use serde_json::Value;

use super::os::Os;

/// One reply, read.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Ok(Value),
    Failed { code: String, message: String },
}

/// The envelope in `output` (the last line that is one, so log lines before
/// it don't matter).
pub fn parse(output: &str) -> Option<Reply> {
    output.lines().rev().find_map(|line| envelope(line.trim()))
}

fn envelope(text: &str) -> Option<Reply> {
    let value: Value = serde_json::from_str(text).ok()?;
    match value.get("ok")?.as_bool()? {
        true => Some(Reply::Ok(value.get("data").cloned().unwrap_or(Value::Null))),
        false => {
            let error = value.get("error");
            let field = |name| {
                error
                    .and_then(|error| error.get(name))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            Some(Reply::Failed {
                code: field("code"),
                message: field("message"),
            })
        }
    }
}

/// How agent-desktop says a permission stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Grants {
    pub accessibility: bool,
    pub screen_recording: bool,
}

/// The grants in `permissions`' or `status`' data
/// (`{accessibility: {state: "granted"}, …}`, under `permissions` for status),
/// as this OS's agent-desktop reports them.
pub fn grants(data: &Value) -> Option<Grants> {
    grants_on(Os::HERE, data)
}

/// [`grants`] on `os`. `not_required` counts as granted (Windows and
/// Linux ask for nothing to read apps, X11 nothing for screenshots); on
/// Linux an `unknown` screen recording is Wayland's, whose portal asks at
/// the first screenshot, so that counts too.
pub fn grants_on(os: Os, data: &Value) -> Option<Grants> {
    let permissions = data.get("permissions").unwrap_or(data);
    let state = |name: &str| -> Option<&str> { permissions.get(name)?.get("state")?.as_str() };
    let ok = |state: &str| matches!(state, "granted" | "not_required");
    Some(Grants {
        accessibility: ok(state("accessibility")?),
        screen_recording: state("screen_recording")
            .is_some_and(|screen| ok(screen) || (os == Os::Linux && screen == "unknown")),
    })
}

/// What agent-desktop suggests for a denied Accessibility (on Linux, why
/// the accessibility bus didn't answer: "Install at-spi2-core …").
pub fn suggestion(data: &Value) -> Option<String> {
    let permissions = data.get("permissions").unwrap_or(data);
    let accessibility = permissions.get("accessibility")?;
    ["suggestion", "message", "detail"]
        .iter()
        .find_map(|key| accessibility.get(*key)?.as_str())
        .map(str::to_owned)
}

/// The status reply an agent's desktop tool call returned, found anywhere in
/// its event stream (`claude -p` stream-json, `codex exec --json`, `opencode
/// run --format json`): each nests the tool's text output differently, so
/// every string is tried as an envelope of `command: "status"`.
pub fn status_in_events(events: &str) -> Option<Value> {
    events.lines().find_map(|line| {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        find_status(&value)
    })
}

fn find_status(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) if text.contains("\"command\"") => {
            let inner: Value = serde_json::from_str(text).ok()?;
            (inner.get("command").and_then(Value::as_str) == Some("status")
                && inner.get("ok").and_then(Value::as_bool) == Some(true))
            .then(|| inner.get("data").cloned())
            .flatten()
        }
        Value::Array(items) => items.iter().find_map(find_status),
        Value::Object(fields) => fields.values().find_map(find_status),
        _ => None,
    }
}

/// A window as `list-windows` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: String,
    pub app: String,
    pub title: String,
    pub visible: bool,
    pub bounds: Option<(f64, f64, f64, f64)>,
}

pub fn windows(data: &Value) -> Vec<Window> {
    let Some(items) = data.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let text = |name| item.get(name).and_then(Value::as_str).map(str::to_owned);
            let bounds = item.get("bounds").and_then(|bounds| {
                let number = |name| bounds.get(name).and_then(Value::as_f64);
                Some((
                    number("x")?,
                    number("y")?,
                    number("width")?,
                    number("height")?,
                ))
            });
            Some(Window {
                id: text("id")?,
                app: text("app_name").unwrap_or_default(),
                title: text("title").unwrap_or_default(),
                visible: item.get("visible").and_then(Value::as_bool).unwrap_or(true),
                bounds,
            })
        })
        .collect()
}

/// The running apps' names in `list-apps`' data.
pub fn apps(data: &Value) -> Vec<String> {
    data.get("apps")
        .and_then(Value::as_array)
        .map(|apps| {
            apps.iter()
                .filter_map(|app| app.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERMISSIONS: &str = r#"{"version":"2.6","ok":true,"command":"permissions","data":{"accessibility":{"state":"granted"},"automation":{"state":"unknown"},"screen_recording":{"state":"denied"}}}"#;

    #[test]
    fn permissions_are_read_from_the_envelope() {
        let Some(Reply::Ok(data)) = parse(&format!("debug: starting\n{PERMISSIONS}\n")) else {
            panic!("no envelope");
        };
        assert_eq!(
            grants(&data),
            Some(Grants {
                accessibility: true,
                screen_recording: false
            })
        );
    }

    #[test]
    fn windows_and_linux_grants_read_as_their_agent_desktop_reports_them() {
        let windows: Value = serde_json::from_str(
            r#"{"accessibility":{"state":"granted"},"automation":{"state":"not_required"},"screen_recording":{"state":"granted"}}"#,
        )
        .unwrap();
        assert_eq!(
            grants_on(Os::Windows, &windows),
            Some(Grants {
                accessibility: true,
                screen_recording: true
            })
        );
        let x11: Value = serde_json::from_str(
            r#"{"permissions":{"accessibility":{"state":"granted"},"screen_recording":{"state":"not_required"}},"supported_surfaces":["window"]}"#,
        )
        .unwrap();
        assert!(grants_on(Os::Linux, &x11).unwrap().screen_recording);
        let wayland: Value = serde_json::from_str(
            r#"{"permissions":{"accessibility":{"state":"denied","suggestion":"Install at-spi2-core"},"screen_recording":{"state":"unknown"}},"supported_surfaces":[]}"#,
        )
        .unwrap();
        assert_eq!(
            grants_on(Os::Linux, &wayland),
            Some(Grants {
                accessibility: false,
                screen_recording: true
            })
        );
        assert_eq!(
            suggestion(&wayland).as_deref(),
            Some("Install at-spi2-core")
        );
        // On macOS an unknown Screen Recording is no grant.
        assert!(!grants_on(Os::Mac, &wayland).unwrap().screen_recording);
    }

    #[test]
    fn a_failure_keeps_its_code_and_message() {
        let output = r#"{"version":"2.6","ok":false,"command":"snapshot","error":{"code":"WINDOW_NOT_FOUND","message":"Application 'Finder' is running but has no matching window"}}"#;
        assert_eq!(
            parse(output),
            Some(Reply::Failed {
                code: "WINDOW_NOT_FOUND".into(),
                message: "Application 'Finder' is running but has no matching window".into()
            })
        );
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn the_status_tool_result_is_found_in_each_agents_events() {
        let status = r#"{\"command\":\"status\",\"data\":{\"permissions\":{\"accessibility\":{\"state\":\"granted\"},\"screen_recording\":{\"state\":\"granted\"}},\"platform\":\"macos\"},\"ok\":true,\"version\":\"2.6\"}"#;
        let claude = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"mcp__agent-desktop__desktop_status","input":{{}}}}]}}}}
{{"type":"user","message":{{"role":"user","content":[{{"tool_use_id":"toolu_1","type":"tool_result","content":"{status}"}}]}}}}"#
        );
        let codex = format!(
            r#"{{"type":"item.completed","item":{{"type":"mcp_tool_call","server":"agent-desktop","tool":"desktop_status","result":{{"content":[{{"type":"text","text":"{status}"}}]}},"status":"completed"}}}}"#
        );
        let opencode = format!(
            r#"{{"type":"tool_use","part":{{"type":"tool","tool":"agent-desktop_desktop_status","state":{{"status":"completed","input":{{}},"output":"{status}"}}}}}}"#
        );
        for events in [claude, codex, opencode] {
            let data = status_in_events(&events).expect("a status result");
            assert_eq!(
                grants(&data),
                Some(Grants {
                    accessibility: true,
                    screen_recording: true
                })
            );
        }
        let refused = r#"{"type":"item.completed","item":{"type":"mcp_tool_call","tool":"desktop_status","result":null,"error":{"message":"MCP tool call requires approval, but approval policy is never"},"status":"failed"}}"#;
        assert_eq!(status_in_events(refused), None);
    }

    #[test]
    fn windows_are_listed() {
        let data: Value = serde_json::from_str(
            r#"[{"accessible":true,"app_name":"Citrix Viewer","bounds":{"height":1440.0,"width":2560.0,"x":-604.0,"y":-1440.0},"id":"w-14569","is_focused":false,"pid":34651,"title":"Citrix Viewer","visible":true}]"#,
        )
        .unwrap();
        let windows = windows(&data);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].app, "Citrix Viewer");
        assert_eq!(windows[0].bounds, Some((-604.0, -1440.0, 2560.0, 1440.0)));
        assert_eq!(windows[0].id, "w-14569");
    }

    #[test]
    fn running_apps_are_named() {
        let data: Value = serde_json::from_str(
            r#"{"apps":[{"bundle_id":"com.apple.finder","name":"Finder","pid":1},{"pid":2}]}"#,
        )
        .unwrap();
        assert_eq!(apps(&data), ["Finder"]);
    }
}
