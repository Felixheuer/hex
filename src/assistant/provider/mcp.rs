//! A small MCP client over stdio for a task's servers (its desktop tool and
//! its browser), so the Claude API backend runs the same tools, with the
//! same schemas, as the agents that load the servers themselves.

use std::io::Write;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::{Control, Process, Stdin};
use crate::assistant::backends::McpServer;

/// One tool the server offers.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// What a tool call returned: MCP content blocks.
#[derive(Clone, Debug)]
pub struct Called {
    pub content: Vec<Value>,
    pub is_error: bool,
}

/// A running MCP server (the guard's, a browser's).
pub struct Client {
    process: Process,
    next_id: u64,
}

impl Client {
    /// Starts `server` and completes the MCP handshake.
    pub fn start(server: &McpServer, workdir: &std::path::Path, control: &Control) -> Result<Self> {
        let mut command = super::command(&server.command, workdir);
        command.args(&server.args);
        let process = Process::spawn(command, Stdin::Keep, control)?;
        let mut client = Self {
            process,
            next_id: 0,
        };
        client.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "hex", "version": crate::VERSION}}),
        )?;
        client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(client)
    }

    /// The server's tools.
    pub fn tools(&mut self) -> Result<Vec<Tool>> {
        let result = self.request("tools/list", json!({}))?;
        Ok(result
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| {
                Some(Tool {
                    name: tool.get("name")?.as_str()?.to_owned(),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                })
            })
            .collect())
    }

    /// Calls `name` with `arguments`.
    pub fn call(&mut self, name: &str, arguments: &Value) -> Result<Called> {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}))?;
        Ok(Called {
            content: result
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            is_error: result.get("isError").and_then(Value::as_bool) == Some(true),
        })
    }

    /// Ends the server.
    pub fn close(self, control: &Control) {
        let _ = self.process.finish(control);
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        let stdin = self
            .process
            .stdin()
            .ok_or_else(|| eyre!("the MCP server's input closed"))?;
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| eyre!("the MCP server stopped: {error}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        while let Some(line) = self.process.next_line() {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
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
                return Err(eyre!("the MCP server refused {method}: {why}"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(eyre!("the MCP server stopped before it answered"))
    }
}
