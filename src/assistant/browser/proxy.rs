//! `hex assistant browser-mcp --task <id>`: agent-browser's own MCP server
//! (`agent-browser mcp`) for one task, relayed line by line over stdio.
//! On the way through every tool call is pinned to one of the task's own
//! browsers ([`Sessions`]), losing the arguments that would show a window,
//! reach another task, keep state, or pass raw flags; the tool list and the
//! server's instructions offer Hex's `session` and `signedIn` instead, plus
//! a tool listing the task's browsers that Hex answers itself. Before a
//! consequential action the autonomy mode is enforced ([`Consent`]), and
//! while the person has the task (Take over) its calls wait ([`Pause`]).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::consent::Consent;
use super::sessions::{self, LIST_TOOL, Sessions};

/// What Hex tells the agent about the browser, after agent-browser's own
/// instructions.
const NOTE: &str = "These browsers are hidden and belong to this task alone: use them \
for anything on the web instead of the person's browser windows. Omit `session` for the \
task's main browser, or name others (\"a\", \"b\", …) to read several pages at once; \
hex_browser_sessions lists them. A browser is clean and light unless its first call says \
signedIn: true, which starts real Chrome with the sites the person signed in to for Hex \
(for web apps such as Gmail or Google Sheets). Nothing is saved.";

/// Serves `task`'s browser tool on stdin and stdout until the agent closes
/// it.
pub fn serve(task: &str) -> Result<()> {
    let path = super::executable().ok_or_else(|| eyre!("agent-browser isn't installed"))?;
    let mut command = super::command(&path, &super::session(task));
    command.arg("mcp");
    let task_id = task.to_owned();
    let consent = Consent::new(
        Box::new(super::PageProbe { path: path.clone() }),
        super::policy(task),
    );
    relay(
        command,
        Sessions::new(task, super::clean_engine()),
        consent,
        Pause::task(task),
        move || super::running(&path, &task_id),
        BufReader::new(std::io::stdin()),
        std::io::stdout(),
    )
}

/// What the agent hears when it acts after the person had the task.
pub const RESUMED: &str = "The person took this task over and may have used its browsers; \
take a fresh snapshot before acting. (The action you asked for was not sent.)";

/// Take over for the browser: while the person has the task, each browser
/// call waits; the first one after Hand back (or one that waited) is not
/// sent, since the page may have changed.
pub struct Pause {
    paused: Box<dyn Fn() -> bool + Send>,
    resumed: Box<dyn Fn() -> Option<String> + Send>,
    /// The hand-back last seen.
    seen: Option<String>,
    poll: Duration,
}

impl Pause {
    /// `task`'s pause, as the guard stores it.
    pub fn task(task: &str) -> Self {
        let (paused, resumed) = (task.to_owned(), task.to_owned());
        Self::new(
            Box::new(move || crate::assistant::guard::paused(&paused)),
            Box::new(move || crate::assistant::guard::resumed(&resumed)),
            Duration::from_millis(250),
        )
    }

    pub fn new(
        paused: Box<dyn Fn() -> bool + Send>,
        resumed: Box<dyn Fn() -> Option<String> + Send>,
        poll: Duration,
    ) -> Self {
        let seen = resumed();
        Self {
            paused,
            resumed,
            seen,
            poll,
        }
    }

    /// Never paused.
    #[cfg(test)]
    pub fn never() -> Self {
        Self::new(Box::new(|| false), Box::new(|| None), Duration::ZERO)
    }

    /// `Err` with the refusal when `tool` must not be sent now.
    fn gate(&mut self, tool: &str) -> Result<(), Value> {
        let waited = (self.paused)();
        while (self.paused)() {
            std::thread::sleep(self.poll);
        }
        let resumed = (self.resumed)();
        let handed_back = resumed != self.seen;
        self.seen = resumed;
        if waited || handed_back {
            return Err(taken_over(tool));
        }
        Ok(())
    }
}

fn taken_over(tool: &str) -> Value {
    let error = json!({
        "ok": false,
        "command": tool,
        "error": {"code": "TAKEN_OVER", "message": RESUMED,
            "details": {"kind": "hex_guard", "retryable": true}},
    });
    json!({
        "content": [{"type": "text", "text": error.to_string()}],
        "structuredContent": error,
        "isError": true,
    })
}

/// What a pending request was, to rewrite or learn from its response.
#[derive(Clone, Debug)]
enum Asked {
    Initialize,
    ToolsList,
    /// A tool call for this session: its result shows the page.
    Call(String),
}

type Pending = Arc<Mutex<HashMap<String, Asked>>>;

/// Starts `server` and relays `input` to it and its replies to `output`,
/// pinned to `sessions` and checked by `consent`, until either side ends.
/// `running` lists the agent-browser sessions that are open, for the list
/// tool.
pub fn relay<W: Write + Send + 'static>(
    mut server: Command,
    sessions: Sessions,
    consent: Consent,
    pause: Pause,
    running: impl Fn() -> Vec<String> + Send + 'static,
    input: impl BufRead + Send + 'static,
    output: W,
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
    let output = Arc::new(Mutex::new(output));
    let write = |output: &Mutex<W>, line: &str| {
        let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
        writeln!(output, "{line}").and_then(|()| output.flush())
    };
    let pending = Pending::default();
    let asked = pending.clone();
    let answers = output.clone();
    let consent = Arc::new(Mutex::new(consent));
    let gate = consent.clone();
    let forward = std::thread::spawn(move || {
        let mut sessions = sessions;
        let mut pause = pause;
        for line in input.lines() {
            let Ok(line) = line else { break };
            if let Some(refused) = held(&line, &mut pause) {
                if write(&answers, &refused).is_err() {
                    break;
                }
                continue;
            }
            let mut consent = gate.lock().unwrap_or_else(|error| error.into_inner());
            let rewritten = rewrite_request(&line, &mut sessions, &mut consent, &running, &asked);
            drop(consent);
            let sent = match rewritten {
                Forward::Server(line) => {
                    writeln!(to_server, "{line}").and_then(|()| to_server.flush())
                }
                Forward::Agent(reply) => write(&answers, &reply),
            };
            if sent.is_err() {
                break;
            }
        }
        // Dropping stdin ends the server once it has answered.
    });
    for line in BufReader::new(from_server).lines() {
        let Ok(line) = line else { break };
        let line = rewrite_response(&line, &pending, &consent);
        if write(&output, &line).is_err() {
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

/// Where a line from the agent goes.
enum Forward {
    Server(String),
    /// Answered by Hex: the list tool, or a call it refused.
    Agent(String),
}

/// The reply refusing `line` when it calls a browser tool while the person
/// has (or just had) the task; Hex's own list tool always passes.
fn held(line: &str, pause: &mut Pause) -> Option<String> {
    let message = serde_json::from_str::<Value>(line).ok()?;
    let calls: Vec<&Value> = match &message {
        Value::Array(batch) => batch.iter().collect(),
        single => vec![single],
    };
    let call = calls.into_iter().find(|request| {
        request.get("method").and_then(Value::as_str) == Some("tools/call")
            && request["params"].get("name").and_then(Value::as_str) != Some(LIST_TOOL)
    })?;
    let tool = call["params"]["name"].as_str().unwrap_or_default();
    let refused = pause.gate(tool).err()?;
    Some(reply(call, refused).to_string())
}

/// One line from the agent, pinned to the task's browsers; `initialize`
/// and `tools/list` are remembered so their answers can be rewritten.
fn rewrite_request(
    line: &str,
    sessions: &mut Sessions,
    consent: &mut Consent,
    running: &dyn Fn() -> Vec<String>,
    pending: &Pending,
) -> Forward {
    let Ok(mut message) = serde_json::from_str::<Value>(line) else {
        return Forward::Server(line.to_owned());
    };
    // Batches went away in MCP 2025-06-18; one is passed on only once
    // every call in it is pinned and allowed.
    if let Some(batch) = message.as_array_mut() {
        for request in batch.iter_mut() {
            if let Err(refused) = pin(request, sessions, consent) {
                return Forward::Agent(reply(request, refused).to_string());
            }
            remember(request, pending);
        }
        return Forward::Server(message.to_string());
    }
    if message.get("method").and_then(Value::as_str) == Some("tools/call")
        && message["params"].get("name").and_then(Value::as_str) == Some(LIST_TOOL)
    {
        let list = sessions.list(&running());
        let reply = json!({"jsonrpc": "2.0", "id": message.get("id"), "result": {
            "content": [{"type": "text", "text": list.to_string()}],
            "structuredContent": list,
            "isError": false,
        }});
        return Forward::Agent(reply.to_string());
    }
    match pin(&mut message, sessions, consent) {
        Ok(()) => {
            remember(&message, pending);
            Forward::Server(message.to_string())
        }
        Err(refused) => Forward::Agent(reply(&message, refused).to_string()),
    }
}

fn remember(request: &Value, pending: &Pending) {
    let asked = match request.get("method").and_then(Value::as_str) {
        Some("initialize") => Asked::Initialize,
        Some("tools/list") => Asked::ToolsList,
        Some("tools/call") => match request["params"]["arguments"]["session"].as_str() {
            Some(session) => Asked::Call(session.to_owned()),
            None => return,
        },
        _ => return,
    };
    if let Some(id) = request.get("id") {
        pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.to_string(), asked);
    }
}

/// A `tools/call` request's arguments, pinned and checked (other messages
/// pass as they are); `Err` is the tool result to answer with instead.
fn pin(request: &mut Value, sessions: &mut Sessions, consent: &mut Consent) -> Result<(), Value> {
    if request.get("method").and_then(Value::as_str) != Some("tools/call") {
        return Ok(());
    }
    let Some(params) = request.get_mut("params").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let tool = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let arguments = params
        .entry("arguments")
        .or_insert_with(|| Value::Object(Default::default()));
    if !arguments.is_object() {
        *arguments = Value::Object(Default::default());
    }
    let Some(arguments) = arguments.as_object_mut() else {
        return Ok(());
    };
    sessions
        .pin(&tool, arguments)
        .map_err(|why| json!({"content": [{"type": "text", "text": why}], "isError": true}))?;
    consent.check(&tool, arguments)
}

/// The JSON-RPC reply to `request` carrying `result`.
fn reply(request: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": request.get("id"), "result": result})
}

/// One line from agent-browser, with Hex's arguments in what it offers;
/// a tool result teaches `consent` what the page shows.
fn rewrite_response(line: &str, pending: &Pending, consent: &Mutex<Consent>) -> String {
    let Ok(mut message) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    let mut changed = false;
    let replies: Vec<&mut Value> = match &mut message {
        Value::Array(batch) => batch.iter_mut().collect(),
        reply @ Value::Object(_) => vec![reply],
        _ => Vec::new(),
    };
    for reply in replies {
        if reply.get("method").is_some() {
            continue;
        }
        let Some(id) = reply.get("id").map(Value::to_string) else {
            continue;
        };
        let asked = pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        let Some(result) = reply.get_mut("result") else {
            continue;
        };
        match asked {
            Some(Asked::Initialize) => instruct(result),
            Some(Asked::ToolsList) => sessions::describe_tools(result),
            Some(Asked::Call(session)) => {
                consent
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .observe(&session, result);
                continue;
            }
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

/// An `initialize` result whose instructions no longer offer the withheld
/// arguments ("Every tool accepts these optional arguments…") and say what
/// these browsers are.
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

    use super::*;
    use crate::assistant::autonomy::{Authorized, Autonomy};
    use crate::assistant::browser::consent::{Inspection, Probe};
    use crate::assistant::browser::sessions::Engine;

    /// A page whose `#send` is Gmail's Send button.
    struct SendButton;

    impl Probe for SendButton {
        fn inspect(&self, _: &str, _: Option<&str>) -> Option<Inspection> {
            None
        }
        fn label(&self, _: &str, selector: &str) -> Option<String> {
            (selector == "#send").then(|| "Send".to_owned())
        }
    }

    #[test]
    fn the_instructions_lose_the_optional_arguments() {
        let mut result = json!({"instructions": "Use the typed agent_browser_* tools. Every tool accepts these optional arguments even when its schema does not list them: session (isolated browser session name)."});
        instruct(&mut result);
        let text = result["instructions"].as_str().unwrap();
        assert!(text.starts_with("Use the typed agent_browser_* tools. These browsers are hidden"));
        assert!(!text.contains("session (isolated"));
    }

    /// Output both the relay and the test hold.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A fake `agent-browser mcp`: answers initialize and tools/list, and
    /// echoes each tools/call it gets back inside its result.
    const FAKE: &str = r#"while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"tools":{}},"instructions":"Use the tools. Every tool accepts these optional arguments: session.","serverInfo":{"name":"agent-browser"}}}' ;;
    *'"method":"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"agent_browser_close","inputSchema":{"type":"object","properties":{"all":{"type":"boolean"},"session":{"type":"string"}}}}]}}' ;;
    *'"method":"tools/call"'*) printf '{"jsonrpc":"2.0","id":9,"result":{"content":[],"echo":%s}}\n' "$line" ;;
  esac
done"#;

    #[cfg(unix)]
    #[test]
    fn the_wrapper_relays_a_task_through_a_fake_agent_browser() {
        let mut server = Command::new("/bin/sh");
        server.args(["-c", FAKE]);
        let call = |id: u64, name: &str, arguments: Value| {
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}})
        };
        let input = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
            call(
                3,
                "agent_browser_open",
                json!({"url": "https://example.com", "session": "a",
                "headed": true, "all": true}),
            ),
            call(4, "agent_browser_open", json!({"session": "../other"})),
            call(5, LIST_TOOL, json!({})),
            call(6, "agent_browser_click", json!({"selector": "#send"})),
        ]
        .map(|message| message.to_string())
        .join("\n");
        let output = Shared::default();
        relay(
            server,
            Sessions::new("t1", Engine::Lightpanda),
            Consent::new(
                Box::new(SendButton),
                Box::new(|| (Autonomy::Smart, Authorized::default())),
            ),
            Pause::never(),
            || vec!["hex-t1_a".to_owned()],
            Cursor::new(input.into_bytes()),
            output.clone(),
        )
        .unwrap();
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        let replies: HashMap<u64, Value> = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .map(|reply| (reply["id"].as_u64().unwrap(), reply))
            .collect();
        assert_eq!(replies.len(), 6, "{text}");
        let refused = &replies[&6]["result"];
        assert_eq!(refused["isError"], true);
        assert_eq!(
            refused["structuredContent"]["error"]["code"],
            "NEEDS_CONFIRMATION"
        );
        assert_eq!(
            refused["structuredContent"]["error"]["details"]["category"],
            "send"
        );
        assert!(
            replies[&1]["result"]["instructions"]
                .as_str()
                .unwrap()
                .starts_with("Use the tools. These browsers are hidden")
        );
        let tools = replies[&2]["result"]["tools"].as_array().unwrap();
        assert!(tools[0]["inputSchema"]["properties"].get("all").is_none());
        assert_eq!(tools[1]["name"], LIST_TOOL);
        assert_eq!(
            replies[&9]["result"]["echo"]["params"]["arguments"],
            json!({"url": "https://example.com", "session": "hex-t1_a",
                "extraArgs": ["--engine", "lightpanda"]})
        );
        assert_eq!(replies[&4]["result"]["isError"], true);
        assert_eq!(
            replies[&5]["result"]["structuredContent"]["browsers"],
            json!([{"session": "a", "signedIn": false, "engine": "lightpanda", "running": true}])
        );
    }

    #[test]
    fn browser_calls_wait_while_taken_over_and_look_again_after() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        let paused = Arc::new(AtomicBool::new(false));
        let resumes = Arc::new(AtomicU64::new(0));
        let (is_paused, resumed) = (paused.clone(), resumes.clone());
        let mut pause = Pause::new(
            Box::new(move || is_paused.load(Ordering::SeqCst)),
            Box::new(move || Some(resumed.load(Ordering::SeqCst).to_string())),
            Duration::from_millis(1),
        );
        let call = |name: &str| {
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": name, "arguments": {}}})
            .to_string()
        };
        assert_eq!(held(&call("agent_browser_click"), &mut pause), None);
        // Handed back while the agent was idle: its next call looks again.
        resumes.store(1, Ordering::SeqCst);
        let refused: Value =
            serde_json::from_str(&held(&call("agent_browser_click"), &mut pause).unwrap()).unwrap();
        assert_eq!(refused["id"], 3);
        assert_eq!(
            refused["result"]["structuredContent"]["error"]["code"],
            "TAKEN_OVER"
        );
        assert_eq!(held(&call("agent_browser_click"), &mut pause), None);
        // Taken over: the call waits until Hand back, then is refused.
        paused.store(true, Ordering::SeqCst);
        let (flag, count) = (paused.clone(), resumes.clone());
        let hand_back = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            count.store(2, Ordering::SeqCst);
            flag.store(false, Ordering::SeqCst);
        });
        assert!(held(&call("agent_browser_type"), &mut pause).is_some());
        hand_back.join().unwrap();
        // Hex's own list tool and other messages are never held.
        paused.store(true, Ordering::SeqCst);
        assert_eq!(held(&call(LIST_TOOL), &mut pause), None);
        assert_eq!(
            held(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &mut pause
            ),
            None
        );
    }
}
