//! `hex assistant browser-mcp --task <id>`: agent-browser's own MCP server
//! (`agent-browser mcp`) for one task, relayed line by line over stdio. Its
//! session is the task's (`AGENT_BROWSER_SESSION`), and on the way through
//! every tool call is pinned to it and loses the arguments that would show a
//! window (`headed`), reach another session (`session`, `namespace`, `all`),
//! keep state (`restore…`), or pass raw flags (`extraArgs`); the tool list
//! and the server's instructions stop offering them.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

/// Tool arguments a task never passes to agent-browser.
pub const WITHHELD_ARGUMENTS: &[&str] = &[
    "session",
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

/// What Hex tells the agent about the browser, after agent-browser's own
/// instructions.
const NOTE: &str = "This browser is hidden and belongs to this task alone: \
use it for anything on the web instead of the person's browser windows. \
Hex chooses its session; it starts with the sites the person signed in to \
for Hex, and nothing it does is saved.";

/// Serves `task`'s browser tool on stdin and stdout until the agent closes
/// it.
pub fn serve(task: &str) -> Result<()> {
    let path = super::executable().ok_or_else(|| eyre!("agent-browser isn't installed"))?;
    let session = super::session(task);
    let mut command = super::command(&path, &session);
    command.arg("mcp");
    relay(
        command,
        &session,
        BufReader::new(std::io::stdin()),
        &mut std::io::stdout(),
    )
}

/// What a pending request was, to rewrite its response.
#[derive(Clone, Copy, Debug)]
enum Asked {
    Initialize,
    ToolsList,
}

type Pending = Arc<Mutex<HashMap<String, Asked>>>;

/// Starts `server` and relays `input` to it and its replies to `output`,
/// rewritten for `session`, until either side ends.
pub fn relay(
    mut server: Command,
    session: &str,
    input: impl BufRead + Send + 'static,
    output: &mut impl Write,
) -> Result<()> {
    server
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = server
        .spawn()
        .map_err(|error| eyre!("agent-browser mcp didn't start: {error}"))?;
    let mut to_server = child.stdin.take().ok_or_else(|| eyre!("no stdin"))?;
    let from_server = child.stdout.take().ok_or_else(|| eyre!("no stdout"))?;
    let pending = Pending::default();
    let asked = pending.clone();
    let session = session.to_owned();
    let forward = std::thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else { break };
            let line = rewrite_request(&line, &session, &asked);
            if writeln!(to_server, "{line}")
                .and_then(|()| to_server.flush())
                .is_err()
            {
                break;
            }
        }
        // Dropping stdin ends the server once it has answered.
    });
    for line in BufReader::new(from_server).lines() {
        let Ok(line) = line else { break };
        let line = rewrite_response(&line, &pending);
        if writeln!(output, "{line}")
            .and_then(|()| output.flush())
            .is_err()
        {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if forward.is_finished() {
        let _ = forward.join();
    }
    Ok(())
}

/// One line from the agent, pinned to `session`; `initialize` and
/// `tools/list` are remembered so their answers can be rewritten.
fn rewrite_request(line: &str, session: &str, pending: &Pending) -> String {
    let Ok(mut message) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    for request in messages(&mut message) {
        let method = request.get("method").and_then(Value::as_str);
        let asked = match method {
            Some("initialize") => Some(Asked::Initialize),
            Some("tools/list") => Some(Asked::ToolsList),
            _ => None,
        };
        if let (Some(asked), Some(id)) = (asked, request.get("id")) {
            pending
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(id.to_string(), asked);
        }
        if method == Some("tools/call") {
            pin_call(request, session);
        }
    }
    message.to_string()
}

/// One line from agent-browser, with the withheld arguments gone from what
/// it offers.
fn rewrite_response(line: &str, pending: &Pending) -> String {
    let Ok(mut message) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    let mut changed = false;
    for response in messages(&mut message) {
        if response.get("method").is_some() {
            continue;
        }
        let Some(id) = response.get("id").map(Value::to_string) else {
            continue;
        };
        let asked = pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        let Some(result) = response.get_mut("result") else {
            continue;
        };
        match asked {
            Some(Asked::Initialize) => instruct(result),
            Some(Asked::ToolsList) => withhold_in_tools(result),
            None => continue,
        }
        changed = true;
    }
    if changed {
        message.to_string()
    } else {
        line.to_owned()
    }
}

/// The JSON-RPC messages in one line: itself, or a batch's.
fn messages(message: &mut Value) -> Vec<&mut Value> {
    match message {
        Value::Array(batch) => batch.iter_mut().filter(|item| item.is_object()).collect(),
        Value::Object(_) => vec![message],
        _ => Vec::new(),
    }
}

/// A `tools/call` request's arguments, pinned to `session`.
pub fn pin_call(request: &mut Value, session: &str) {
    let Some(params) = request.get_mut("params").and_then(Value::as_object_mut) else {
        return;
    };
    let arguments = params
        .entry("arguments")
        .or_insert_with(|| Value::Object(Default::default()));
    if !arguments.is_object() {
        *arguments = Value::Object(Default::default());
    }
    let Some(arguments) = arguments.as_object_mut() else {
        return;
    };
    for name in WITHHELD_ARGUMENTS {
        arguments.remove(*name);
    }
    arguments.insert("session".into(), session.into());
}

/// A `tools/list` result without the withheld arguments in any schema.
pub fn withhold_in_tools(result: &mut Value) {
    let Some(tools) = result.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for tool in tools {
        let Some(schema) = tool.get_mut("inputSchema") else {
            continue;
        };
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            for name in WITHHELD_ARGUMENTS {
                properties.remove(*name);
            }
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| {
                name.as_str()
                    .is_none_or(|name| !WITHHELD_ARGUMENTS.contains(&name))
            });
        }
    }
}

/// An `initialize` result whose instructions no longer offer the withheld
/// arguments ("Every tool accepts these optional arguments…") and say what
/// this browser is.
pub fn instruct(result: &mut Value) {
    let Some(result) = result.as_object_mut() else {
        return;
    };
    let theirs = result
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let kept = theirs
        .split_once("Every tool accepts")
        .map_or(theirs, |(kept, _)| kept)
        .trim();
    let instructions = if kept.is_empty() {
        NOTE.to_owned()
    } else {
        format!("{kept} {NOTE}")
    };
    result.insert("instructions".into(), instructions.into());
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    #[test]
    fn a_call_is_pinned_to_the_tasks_session() {
        let mut call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "agent_browser_open",
            "arguments": {"url": "https://example.com", "session": "default",
                "headed": true, "extraArgs": ["--headed"], "restore": true, "namespace": "x"}}});
        pin_call(&mut call, "hex-t1");
        assert_eq!(
            call["params"]["arguments"],
            json!({"url": "https://example.com", "session": "hex-t1"})
        );
        let mut close = json!({"method": "tools/call", "params": {"name": "agent_browser_close",
            "arguments": {"all": true}}});
        pin_call(&mut close, "hex-t1");
        assert_eq!(close["params"]["arguments"], json!({"session": "hex-t1"}));
        let mut bare = json!({"method": "tools/call", "params": {"name": "agent_browser_get_url"}});
        pin_call(&mut bare, "hex-t1");
        assert_eq!(bare["params"]["arguments"], json!({"session": "hex-t1"}));
    }

    #[test]
    fn the_tool_list_stops_offering_withheld_arguments() {
        let mut result = json!({"tools": [{"name": "agent_browser_open", "inputSchema": {
            "type": "object", "required": ["session", "url"],
            "properties": {"url": {"type": "string"}, "session": {"type": "string"},
                "headed": {"type": "boolean"}, "extraArgs": {"type": "array"}}}}]});
        withhold_in_tools(&mut result);
        let schema = &result["tools"][0]["inputSchema"];
        assert_eq!(schema["properties"], json!({"url": {"type": "string"}}));
        assert_eq!(schema["required"], json!(["url"]));
    }

    #[test]
    fn the_instructions_lose_the_optional_arguments() {
        let mut result = json!({"instructions": "Use the typed agent_browser_* tools. Every tool accepts these optional arguments even when its schema does not list them: session (isolated browser session name)."});
        instruct(&mut result);
        let text = result["instructions"].as_str().unwrap();
        assert!(text.starts_with("Use the typed agent_browser_* tools. This browser is hidden"));
        assert!(!text.contains("session (isolated"));
    }

    /// A fake `agent-browser mcp`: answers initialize and tools/list, and
    /// echoes each tools/call it gets back inside its result.
    const FAKE: &str = r#"while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"tools":{}},"instructions":"Use the tools. Every tool accepts these optional arguments: session.","serverInfo":{"name":"agent-browser"}}}' ;;
    *'"method":"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"agent_browser_close","inputSchema":{"type":"object","properties":{"all":{"type":"boolean"},"session":{"type":"string"}}}}]}}' ;;
    *'"method":"tools/call"'*) printf '{"jsonrpc":"2.0","id":3,"result":{"content":[],"echo":%s}}\n' "$line" ;;
  esac
done"#;

    #[test]
    fn the_wrapper_relays_a_session_through_a_fake_agent_browser() {
        let mut server = Command::new("/bin/sh");
        server.args(["-c", FAKE]);
        let input = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "agent_browser_close", "arguments": {"all": true, "session": "default"}}}),
        ]
        .map(|message| message.to_string())
        .join("\n");
        let mut output = Vec::new();
        relay(
            server,
            "hex-t1",
            Cursor::new(input.into_bytes()),
            &mut output,
        )
        .unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 3, "{replies:?}");
        let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
        assert!(instructions.starts_with("Use the tools. This browser is hidden"));
        assert_eq!(
            replies[1]["result"]["tools"][0]["inputSchema"]["properties"],
            json!({})
        );
        assert_eq!(
            replies[2]["result"]["echo"]["params"]["arguments"],
            json!({"session": "hex-t1"})
        );
    }
}
