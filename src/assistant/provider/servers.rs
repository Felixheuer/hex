//! A task's MCP servers in each backend's config format: its desktop tool
//! under agent-desktop's name (the guard forwards agent-desktop's tools under
//! the same names, so `desktop_*` reads the same either way), and its hidden
//! browser under `browser`.

use serde_json::{Map, Value, json};

use super::{DESKTOP_TOOLS, Run, WITHHELD_TOOLS};
use crate::assistant::backends::{MCP_NAME, McpServer};

/// The hidden browser's server name; its tools are `mcp__browser__*`
/// (Claude Code), `browser_*` (OpenCode), and their own names (Codex).
pub const BROWSER: &str = "browser";

/// The servers `run` gets, by name.
pub fn of(run: &Run) -> Vec<(&'static str, &McpServer)> {
    let mut servers = Vec::new();
    if let Some(desktop) = &run.desktop {
        servers.push((MCP_NAME, &desktop.server));
    }
    if let Some(browser) = &run.browser {
        servers.push((BROWSER, browser));
    }
    servers
}

/// `claude --mcp-config <this> --strict-mcp-config`.
pub fn claude_config(run: &Run) -> Option<String> {
    let servers = of(run);
    if servers.is_empty() {
        return None;
    }
    let mut map = Map::new();
    for (name, server) in servers {
        map.insert(
            name.to_owned(),
            json!({
                "type": "stdio",
                "command": server.command,
                "args": server.args,
                "env": { "PATH": server.path_env },
            }),
        );
    }
    Some(json!({ "mcpServers": map }).to_string())
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
    let mut args = Vec::new();
    for (name, server) in of(run) {
        let key = format!("mcp_servers.{name}");
        let toml = |value: Value| value.to_string();
        let mut values = vec![
            format!(
                "{key}.command={}",
                toml(server.command.to_string_lossy().into())
            ),
            format!("{key}.args={}", toml(server.args.clone().into())),
            format!("{key}.env.PATH={}", toml(server.path_env.clone().into())),
            format!("{key}.default_tools_approval_mode=\"approve\""),
        ];
        if name == MCP_NAME {
            values.push(format!(
                "{key}.enabled_tools={}",
                toml(DESKTOP_TOOLS.to_vec().into())
            ));
        }
        args.extend(values.into_iter().flat_map(|value| ["-c".to_owned(), value]));
    }
    args
}

/// The `mcp` entry of an OpenCode config, if `run` has servers.
pub fn opencode_config(run: &Run) -> Option<Value> {
    let servers = of(run);
    if servers.is_empty() {
        return None;
    }
    let mut map = Map::new();
    for (name, server) in servers {
        let mut command = vec![server.command.to_string_lossy().into_owned()];
        command.extend(server.args.iter().cloned());
        map.insert(
            name.to_owned(),
            json!({
                "type": "local",
                "command": command,
                "environment": { "PATH": server.path_env },
            }),
        );
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
        assert_eq!(claude["mcpServers"]["agent-desktop"]["args"][2], "s1");
        assert_eq!(claude["mcpServers"]["browser"]["args"][2], "t1");
        let (allowed, withheld) = claude_tools(&run);
        assert_eq!(allowed, ["mcp__agent-desktop", "mcp__browser"]);
        assert!(withheld.contains(&"mcp__agent-desktop__desktop_session_end".to_owned()));

        let codex = codex_args(&run);
        assert!(codex.contains(&r#"mcp_servers.browser.args=["mcp","--session","t1"]"#.to_owned()));
        assert!(
            codex
                .iter()
                .any(|arg| arg.starts_with("mcp_servers.agent-desktop.enabled_tools="))
        );
        assert!(
            !codex
                .iter()
                .any(|arg| arg.starts_with("mcp_servers.browser.enabled_tools="))
        );

        let opencode = opencode_config(&run).unwrap();
        assert_eq!(
            opencode["browser"]["command"],
            json!(["/opt/homebrew/bin/agent-browser", "mcp", "--session", "t1"])
        );
        assert_eq!(opencode["agent-desktop"]["type"], "local");
    }
}
