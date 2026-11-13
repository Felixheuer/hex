//! The guard's `desktop_status` reply as setup reads it: the permissions
//! Cua Driver acts with, found in an agent's event stream when the agent
//! called the tool.

use serde_json::Value;

/// How a permission stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Grants {
    pub accessibility: bool,
    pub screen_recording: bool,
}

/// The grants in `desktop_status`' data
/// (`{permissions: {accessibility: {state: "granted"}, …}}`).
pub fn grants(data: &Value) -> Option<Grants> {
    let permissions = data.get("permissions").unwrap_or(data);
    let granted = |name: &str| -> Option<bool> {
        let state = permissions.get(name)?.get("state")?.as_str()?;
        Some(state == "granted")
    };
    Some(Grants {
        accessibility: granted("accessibility")?,
        screen_recording: granted("screen_recording").unwrap_or(false),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_are_read_from_the_status() {
        let data = serde_json::json!({"driver": "cua-driver", "permissions": {
            "accessibility": {"state": "granted"}, "screen_recording": {"state": "denied"}}});
        assert_eq!(
            grants(&data),
            Some(Grants {
                accessibility: true,
                screen_recording: false
            })
        );
        assert_eq!(grants(&serde_json::json!({})), None);
    }

    #[test]
    fn the_status_tool_result_is_found_in_each_agents_events() {
        let status = r#"{\"command\":\"status\",\"data\":{\"driver\":\"cua-driver\",\"permissions\":{\"accessibility\":{\"state\":\"granted\"},\"screen_recording\":{\"state\":\"granted\"}}},\"ok\":true,\"version\":\"cua-driver\"}"#;
        let claude = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"mcp__desktop__desktop_status","input":{{}}}}]}}}}
{{"type":"user","message":{{"role":"user","content":[{{"tool_use_id":"toolu_1","type":"tool_result","content":"{status}"}}]}}}}"#
        );
        let codex = format!(
            r#"{{"type":"item.completed","item":{{"type":"mcp_tool_call","server":"desktop","tool":"desktop_status","result":{{"content":[{{"type":"text","text":"{status}"}}]}},"status":"completed"}}}}"#
        );
        let opencode = format!(
            r#"{{"type":"tool_use","part":{{"type":"tool","tool":"desktop_desktop_status","state":{{"status":"completed","input":{{}},"output":"{status}"}}}}}}"#
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
}
