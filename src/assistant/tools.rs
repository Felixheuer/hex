//! Hex's own small MCP server for a task's agent (`hex assistant tools-mcp
//! --task <id>`): `hex_remember` adds a fact to the person's memory note,
//! and `hex_schedule`, `hex_schedules`, `hex_unschedule` manage scheduled
//! jobs. Newline-delimited JSON-RPC on stdio, like the other servers.
//!
//! OWNER: brain.

use std::io::{BufRead, Write};

use color_eyre::Result;
use serde_json::{Value, json};

use crate::assistant::types::Lane;
use crate::assistant::{memory, schedules};

/// The server's name in each agent's config.
pub const NAME: &str = "hex";

/// The tool definitions.
pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "hex_remember",
            "description": "Remember one durable fact the person told you, for every later task: their company, website, a colleague's name and email, the tone or language they like. One short fact per call. Never passwords, keys, tokens, card or account numbers.",
            "inputSchema": {"type": "object", "properties": {
                "fact": {"type": "string", "description": "The fact, as one short sentence."}
            }, "required": ["fact"]},
        }),
        json!({
            "name": "hex_schedule",
            "description": format!(
                "Schedule a job to run by itself later or repeatedly (\"every morning at 8, …\"), in this {}'s local time. Each run is a background job doing `task`. Tell the person the schedule and its next runs in your answer.",
                crate::assistant::backends::os::Os::HERE.computer()
            ),
            "inputSchema": {"type": "object", "properties": {
                "when": {"type": "string", "description": "A cron line (minute hour day month weekday, e.g. \"0 8 * * 1-5\") or an RRULE (e.g. \"FREQ=WEEKLY;BYDAY=MO;BYHOUR=9\")."},
                "task": {"type": "string", "description": "What to do each time, as the person would say it."},
                "lane": {"type": "string", "enum": ["background", "here"], "description": "Always background in practice."}
            }, "required": ["when", "task"]},
        }),
        json!({
            "name": "hex_schedules",
            "description": "List the scheduled jobs with their ids and next runs.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "hex_unschedule",
            "description": "Remove a scheduled job by its id (from hex_schedules).",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"}
            }, "required": ["id"]},
        }),
    ]
}

/// Runs one tool; its text, or why it failed.
pub fn call(name: &str, arguments: &Value) -> Result<String, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name} needs “{key}”"))
    };
    match name {
        "hex_remember" => {
            if !memory_on() {
                return Err("The person turned memory off; nothing is remembered.".into());
            }
            memory::remember(&text("fact")?)
                .map(|line| format!("Remembered: {line}"))
                .map_err(|error| format!("{error:#}"))
        }
        "hex_schedule" => {
            let lane = match arguments.get("lane").and_then(Value::as_str) {
                Some("here") => Lane::Here,
                _ => Lane::Background,
            };
            let schedule = schedules::add(&text("when")?, &text("task")?, lane)
                .map_err(|error| format!("{error:#}"))?;
            let next = schedules::next_runs(&schedule.when, 3)
                .unwrap_or_default()
                .into_iter()
                .map(schedules::describe)
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "Scheduled {} ({}): “{}”. Next runs: {next}.",
                schedule.id, schedule.when, schedule.task
            ))
        }
        "hex_schedules" => {
            let list = schedules::list();
            if list.is_empty() {
                return Ok("No scheduled jobs.".into());
            }
            Ok(list
                .iter()
                .map(|schedule| {
                    let next = schedule
                        .next_due()
                        .map_or_else(|| "never".into(), schedules::describe);
                    format!(
                        "{} [{}] {}: “{}”, next {next}",
                        schedule.id,
                        if schedule.enabled { "on" } else { "off" },
                        schedule.when,
                        schedule.task
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "hex_unschedule" => {
            let id = text("id")?;
            if schedules::remove(&id) {
                Ok(format!("Removed the schedule {id}."))
            } else {
                Err(format!("There is no schedule {id}."))
            }
        }
        other => Err(format!("There is no tool “{other}”.")),
    }
}

fn memory_on() -> bool {
    crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.memory)
        .or_else(|| {
            crate::app_settings::AppSettings::read()
                .ok()
                .map(|settings| settings.assistant.memory)
        })
        .unwrap_or(true)
}

/// Serves the tools on stdio until the agent disconnects.
pub fn serve(task: &str) -> Result<()> {
    let _ = task;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(reply) = answer(&message) {
            let mut text = serde_json::to_vec(&reply)?;
            text.push(b'\n');
            stdout.write_all(&text)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// The reply to one JSON-RPC message; `None` for a notification.
pub fn answer(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params
                .get("protocolVersion")
                .cloned()
                .unwrap_or_else(|| json!("2025-06-18")),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "hex", "version": crate::VERSION},
        }),
        "tools/list" => json!({"tools": definitions()}),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call(name, &arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            json!({"content": [{"type": "text", "text": text}], "isError": is_error})
        }
        "ping" => json!({}),
        other => {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {other}")}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_speaks_mcp_and_refuses_unknown_tools_and_secrets() {
        let init = answer(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}}))
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "hex");
        assert!(
            answer(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none()
        );
        let list = answer(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(
            names,
            [
                "hex_remember",
                "hex_schedule",
                "hex_schedules",
                "hex_unschedule"
            ]
        );
        let secret = answer(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "hex_remember", "arguments": {"fact": "My password is hunter2"}}}))
        .unwrap();
        assert_eq!(secret["result"]["isError"], true);
        let bad = call("hex_schedule", &json!({"when": "whenever", "task": "x"})).unwrap_err();
        assert!(bad.contains("isn't a schedule"), "{bad}");
        assert!(call("hex_nothing", &json!({})).is_err());
    }
}
