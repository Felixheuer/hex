//! A task's MCP servers in each backend's config format: its desktop tool
//! under agent-desktop's name (the guard forwards agent-desktop's tools under
//! the same names, plus `desktop_claim_window`, so `desktop_*` reads the same
//! either way), and its hidden browser under `agent-browser`.

use serde_json::{Map, Value};

use super::{DESKTOP_TOOLS, Run, WITHHELD_TOOLS};
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

/// Claude Code's allowed and withheld tools for `run`'s servers.
pub fn claude_tools(run: &Run) -> (Vec<String>, Vec<String>) {
    let mut allowed = Vec::new();
    let mut withheld = Vec::new();
    for (name, _) in of(run) {
        allowed.push(format!("mcp__{name}"));
        if name == MCP_NAME {
            withheld.extend(
                WITHHELD_TOOLS
                    .iter()
                    .map(|tool| format!("mcp__{name}__{tool}")),
            );
        }
    }
    (allowed, withheld)
}

/// `-c` overrides that add `run`'s servers to `codex exec`, approved up
/// front: the desktop with only `DESKTOP_TOOLS`, the browser with all of its.
pub fn codex_args(run: &Run) -> Vec<String> {
    of(run)
        .into_iter()
        .flat_map(|(name, server)| {
            let tools: &[&str] = if name == MCP_NAME { DESKTOP_TOOLS } else { &[] };
            server.codex_args_as(name, tools)
        })
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
        run.desktop = Some(Desktop::fixture(Some("s1"), "hex-2"));
        run.browser = Some(browser());

        let claude: Value = serde_json::from_str(&claude_config(&run).unwrap()).unwrap();
        assert_eq!(claude["mcpServers"]["desktop"]["args"][2], "s1");
        assert_eq!(claude["mcpServers"]["agent-browser"]["args"][2], "t1");
        let (allowed, withheld) = claude_tools(&run);
        assert_eq!(allowed, ["mcp__desktop", "mcp__agent-browser"]);
        assert!(withheld.contains(&"mcp__desktop__desktop_session_end".to_owned()));

        let codex = codex_args(&run);
        assert!(
            codex
                .contains(&r#"mcp_servers.agent-browser.args=["mcp","--session","t1"]"#.to_owned())
        );
        assert!(codex.iter().any(
            |arg| arg.starts_with("mcp_servers.desktop.enabled_tools=")
                && arg.contains("desktop_claim_window")
                && arg.contains("hex_installed_apps")
                && arg.contains("hex_open_app")
        ));
        assert!(
            !codex
                .iter()
                .any(|arg| arg.starts_with("mcp_servers.agent-browser.enabled_tools="))
        );

        let opencode = opencode_config(&run).unwrap();
        assert_eq!(
            opencode["agent-browser"]["command"],
            serde_json::json!(["/opt/homebrew/bin/agent-browser", "mcp", "--session", "t1"])
        );
        assert_eq!(opencode["desktop"]["type"], "local");
    }
}
