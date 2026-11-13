//! A task's MCP servers in each backend's config format: its desktop tool
//! (the guard, which serves Cua Driver's tools as `desktop_*` plus Agent-Desk's
//! own), its hidden browser under `agent-browser`, and Agent-Desk's
//! tools. Each server lists exactly the tools the task may use, so every
//! backend gets all of them and none keeps an allowlist of its own
//! (`guard::proxy::tools`, `tools::definitions_for`).

use serde_json::{Map, Value};

use super::Run;
use crate::assistant::backends::{MCP_NAME, McpServer};

/// The hidden browser's server name; its tools are `mcp__agent-browser__*`
/// (Claude Code) and `agent-browser_*` (OpenCode).
pub const BROWSER: &str = crate::assistant::browser::MCP_NAME;

/// The servers `run` gets, by name.
pub fn of(run: &Run) -> Vec<(&'static str, &McpServer)> {
    let mut servers = Vec::new();
    if let Some(desktop) = &run.desktop {
        servers.push((MCP_NAME, &desktop.server));
    }
    if let Some(browser) = &run.browser {
        servers.push((BROWSER, browser));
    }
    if let Some(tools) = &run.tools {
        servers.push((crate::assistant::tools::NAME, tools));
    }
    servers
}

/// `claude --mcp-config <this> --strict-mcp-config`: every server in one
/// `mcpServers` map.
pub fn claude_config(run: &Run) -> Option<String> {
    let servers = of(run);
    if servers.is_empty() {
        return None;
    }
    let mut map = Map::new();
    for (name, server) in servers {
        map.insert(name.to_owned(), server.claude_entry());
    }
    Some(serde_json::json!({ "mcpServers": map }).to_string())
}

/// Claude Code's allowed tools for `run`'s servers: each server whole.
pub fn claude_tools(run: &Run) -> Vec<String> {
    of(run)
        .into_iter()
        .map(|(name, _)| format!("mcp__{name}"))
        .collect()
}

/// `-c` overrides that add `run`'s servers to `codex exec`, approved up
/// front, each with every tool it lists.
pub fn codex_args(run: &Run) -> Vec<String> {
    of(run)
        .into_iter()
        .flat_map(|(name, server)| server.codex_args_as(name, &[]))
        .collect()
}

/// The `mcp` entry of an OpenCode config, if `run` has servers.
pub fn opencode_config(run: &Run) -> Option<Value> {
    let servers = of(run);
    if servers.is_empty() {
        return None;
    }
    let mut map = Map::new();
    for (name, server) in servers {
        if let Value::Object(entry) = server.opencode_config_as(name) {
            map.extend(entry);
        }
    }
    Some(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::AssistantBackend;
    use crate::assistant::provider::Desktop;

    fn browser() -> McpServer {
        McpServer {
            command: "/opt/homebrew/bin/agent-browser".into(),
            args: vec!["mcp".into(), "--session".into(), "t1".into()],
            path_env: "/usr/bin".into(),
        }
    }

    #[test]
    fn every_backend_gets_the_desktop_and_the_browser() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        assert_eq!(claude_config(&run), None);
        assert!(codex_args(&run).is_empty());
        run.desktop = Some(Desktop::fixture());
        run.browser = Some(browser());

        let claude: Value = serde_json::from_str(&claude_config(&run).unwrap()).unwrap();
        assert_eq!(claude["mcpServers"]["desktop"]["args"][2], "--spec");
        assert_eq!(claude["mcpServers"]["agent-browser"]["args"][2], "t1");
        assert_eq!(claude_tools(&run), ["mcp__desktop", "mcp__agent-browser"]);

        let codex = codex_args(&run);
        assert!(
            codex
                .contains(&r#"mcp_servers.agent-browser.args=["mcp","--session","t1"]"#.to_owned())
        );
        // The guard lists exactly the task's tools: no backend narrows them.
        assert!(!codex.iter().any(|arg| arg.contains(".enabled_tools=")));

        let opencode = opencode_config(&run).unwrap();
        assert_eq!(
            opencode["agent-browser"]["command"],
            serde_json::json!(["/opt/homebrew/bin/agent-browser", "mcp", "--session", "t1"])
        );
        assert_eq!(opencode["desktop"]["type"], "local");
    }
}
