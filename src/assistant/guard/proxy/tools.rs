//! The tools a task's guard offers: the driver's, less those its lane may
//! never use, plus Agent-Desk's own. One definition for every backend: Claude
//! Code, Codex, OpenCode, and the Claude API all see exactly this list, so
//! no backend keeps an allowlist of its own (`provider::servers`).
//!
//! Each job is done by one tool: apps open with `hex_open_app` (never
//! `desktop_launch`, which the guard still runs for it), and an app's
//! scripting is offered only where the task may act through it.

use serde_json::{Map, Value, json};

use super::Config;
use crate::assistant::guard::policy::{self, Kind};
use crate::assistant::types::Lane;

/// The guard's MCP instructions: a pointer, so the system prompt is the one
/// place with the rules (the Claude API never reads MCP instructions).
pub const RULES: &str = "Agent-Desk guards these tools: each call is checked against this task's \
windows and the person's settings. Your system prompt has the rules. A refusal is a tool error \
with a code and says what to do instead.";

/// The tools a task with `config` gets, from the driver's `tools`.
pub fn offered(config: &Config, mut tools: Vec<Value>) -> Vec<Value> {
    let background = config.lane == Lane::Background;
    let physical = config.physical_input;
    tools.retain(|tool| {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
        match policy::kind(name, &Map::new()) {
            Kind::Physical => physical,
            Kind::Clipboard => !background || physical,
            // Background first: a Background task brings a window forward
            // (briefly) only with full control.
            Kind::FocusWindow => !background || config.front_window,
            // The whole screen shows the person's windows: full control.
            Kind::Screen => config.front_window,
            Kind::Free
            | Kind::Observe
            | Kind::Act
            | Kind::Press
            | Kind::CloseApp
            | Kind::Batch
            | Kind::Script => true,
            // Agent-Desk's `hex_open_app` opens apps; the rest are refused or Agent-Desk's.
            _ => false,
        }
    });
    // Apps reach: a Background task, or full control (or allowed to open
    // apps) in the Here lane.
    let reach = background || config.open_apps || config.front_window;
    if background {
        tools.push(claim_tool());
    }
    tools.extend(super::apps::tools(reach));
    if reach {
        // The desk is the Here lane's: windows a quick turn opened stay
        // there until the person is done with them.
        if !background {
            tools.push(super::desk::tool());
        }
        tools.push(super::actions::capabilities_tool());
        tools.push(super::actions::app_action_tool());
    }
    tools.push(super::actions::read_window_tool());
    tools.push(super::vision::shot_tool());
    tools.push(super::vision::input_tool());
    tools.extend(super::skills::tools());
    tools
}

/// The names of the tools a task with `config` gets when Cua Driver serves
/// the desktop: what the system prompt may name.
pub fn offered_names(config: &Config) -> Vec<String> {
    offered(config, crate::assistant::guard::cua::tools())
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn claim_tool() -> Value {
    json!({
        "name": policy::CLAIM_TOOL,
        "description": "Agent-Desk: take a window that's on screen and free (no other task holds it, \
            and it isn't the window in front of the person) so this task may read and operate \
            it. Windows this task opens with hex_open_app are its own already.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "window_id": {"type": "string", "description": "The window's id from desktop_list_windows (w-1234)"}
            },
            "required": ["window_id"],
        },
    })
}
