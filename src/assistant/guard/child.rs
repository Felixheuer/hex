//! agent-desktop behind the guard: `agent-desktop mcp` with the task's
//! `--session`/`--agent-id` (and `--headed` when real input is allowed),
//! spoken to over stdio. Calls are forwarded with their arguments as given,
//! so the tools keep agent-desktop's names and schemas exactly.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

use serde_json::{Map, Value, json};

use crate::assistant::backends::run;

/// What the guard forwards to; a fake in tests.
pub trait Desktop: Send {
    /// agent-desktop's tools as it lists them (MCP `Tool` objects).
    fn tools(&mut self) -> Result<Vec<Value>, String>;
    /// Calls `tool`; the MCP `CallToolResult` as it came.
    fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String>;
    /// agent-desktop's own instructions, once started.
    fn instructions(&mut self) -> Option<String> {
        None
    }
}

/// A running `agent-desktop mcp`, started on first use and again if it ends.
pub struct AgentDesktop {
    program: PathBuf,
    args: Vec<String>,
    /// Environment beside Hex's agent environment (Cua Driver's settings).
    env: Vec<(String, String)>,
    running: Option<Running>,
    instructions: Option<String>,
    next_id: u64,
}

struct Running {
    child: Child,
    tree: run::Tree,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl AgentDesktop {
    pub fn new(program: PathBuf, flags: Vec<String>) -> Self {
        let mut args = vec!["mcp".to_owned()];
        args.extend(flags);
        Self::server(program, args, Vec::new())
    }

    /// Any desktop driver's MCP server over stdio: `program` with `args`
    /// (its MCP subcommand included) and `env`.
    pub fn server(program: PathBuf, args: Vec<String>, env: Vec<(String, String)>) -> Self {
        Self {
            program,
            args,
            env,
            running: None,
            instructions: None,
            next_id: 0,
        }
    }

    fn start(&mut self) -> Result<(), String> {
        if let Some(running) = &mut self.running
            && matches!(running.child.try_wait(), Ok(None))
        {
            return Ok(());
        }
        self.running = None;
        let mut command = crate::assistant::backends::search::command(&self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Its own tree, ended with it (agent-desktop's helpers too); not
        // tied to this thread, which may hand the guard on.
        run::own_group(&mut command, false);
        let mut child = command
            .spawn()
            .map_err(|error| format!("agent-desktop didn't start: {error}"))?;
        let tree = run::Tree::of(&child);
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err("agent-desktop didn't start".into());
        };
        self.running = Some(Running {
            child,
            tree,
            stdin,
            stdout: BufReader::new(stdout),
        });
        let hello = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "hex-guard", "version": crate::VERSION}}),
        )?;
        self.instructions = hello
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let running = self.running.as_mut().ok_or("agent-desktop isn't running")?;
        let mut line = message.to_string().into_bytes();
        line.push(b'\n');
        let written = running
            .stdin
            .write_all(&line)
            .and_then(|()| running.stdin.flush());
        written.map_err(|error| {
            self.running = None;
            format!("agent-desktop stopped: {error}")
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let running = self.running.as_mut().ok_or("agent-desktop isn't running")?;
        let mut line = String::new();
        loop {
            line.clear();
            match running.stdout.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    self.running = None;
                    return Err("agent-desktop stopped before it answered".into());
                }
                Ok(_) => {}
            }
            let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let why = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(format!("agent-desktop refused {method}: {why}"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl Desktop for AgentDesktop {
    fn tools(&mut self) -> Result<Vec<Value>, String> {
        self.start()?;
        let listed = self.request("tools/list", json!({}))?;
        Ok(listed
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
        self.start()?;
        self.request("tools/call", json!({"name": tool, "arguments": args}))
    }

    fn instructions(&mut self) -> Option<String> {
        self.start().ok()?;
        self.instructions.clone()
    }
}

impl Drop for AgentDesktop {
    fn drop(&mut self) {
        if let Some(mut running) = self.running.take() {
            running.tree.kill_with(&mut running.child);
        }
    }
}

/// The agent-desktop envelope in a tool result (`structuredContent`, else
/// the first text block that parses as one).
pub fn envelope(result: &Value) -> Option<Value> {
    if let Some(structured) = result.get("structuredContent")
        && structured.get("ok").is_some()
    {
        return Some(structured.clone());
    }
    result
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|block| block.get("text")?.as_str())
        .find_map(|text| {
            let value: Value = serde_json::from_str(text).ok()?;
            value.get("ok").is_some().then_some(value)
        })
}

/// The `data` of a successful envelope in `result`.
pub fn data(result: &Value) -> Option<Value> {
    let envelope = envelope(result)?;
    (envelope.get("ok")? == true).then(|| envelope.get("data").cloned().unwrap_or(Value::Null))
}

/// A window as `list-windows` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: String,
    pub app: String,
    pub title: String,
    pub pid: Option<i64>,
    pub focused: bool,
    /// On screen (not minimized, not an app's invisible helper window).
    pub visible: bool,
    pub minimized: bool,
    pub bounds: Option<(f64, f64, f64, f64)>,
}

impl Window {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.bounds.is_some_and(|(left, top, width, height)| {
            x >= left && x < left + width && y >= top && y < top + height
        })
    }
}

/// The windows in `list-windows`' data.
pub fn windows(data: &Value) -> Vec<Window> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let text = |name| item.get(name).and_then(Value::as_str).map(str::to_owned);
            let flag = |name, default| item.get(name).and_then(Value::as_bool).unwrap_or(default);
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
                pid: item.get("pid").and_then(Value::as_i64),
                focused: flag("is_focused", false),
                visible: flag("visible", true) && !flag("minimized", false),
                minimized: flag("minimized", false),
                bounds,
            })
        })
        .collect()
}
