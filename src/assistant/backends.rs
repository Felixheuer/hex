//! Finding the agents installed on this computer (Claude Code, Codex, OpenCode)
//! and agent-desktop, the desktop tool every task uses. A GUI app does not
//! inherit the person's shell PATH, so this looks where installers put them
//! and asks the login shell (`search`), and gives every agent it starts that
//! PATH (`agent_env`), so a Node-based agent finds `node`.
//!
//! Setup proves each piece works with `check`'s self-tests, run from Hex so
//! macOS judges Hex's own permissions: an agent Hex starts, and the
//! agent-desktop it starts, act with Hex.app's Accessibility and Screen
//! Recording, not Terminal's.
//!
//! OWNER: hands.

mod check;
pub(in crate::assistant) mod desktop;
#[cfg(debug_assertions)]
pub mod doctor;
mod install;
mod keys;
pub(in crate::assistant) mod launch;
pub mod os;
mod providers;
mod recommend;
pub(in crate::assistant) mod run;
pub(in crate::assistant) mod search;
pub(in crate::assistant) mod tree;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::app_settings::AssistantBackend;

pub use check::{
    Check, CheckResult, CheckState, Fix, opencode_recent_model, self_test, self_tests,
};
pub use install::{Installers, Tool, apply_fix, apply_fix_with, install_command};
pub use providers::{
    Provider, agent_env_for, merge_config, opencode_model, opencode_provider_config, providers,
    set_provider_key, verify_provider_key,
};
pub use recommend::recommendations;

/// One agent found on this computer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Found {
    pub backend: AssistantBackend,
    /// The executable (`None` for `ClaudeApi`, and for OpenCode when only its
    /// service is running).
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Ready to take a task: installed and, as far as Hex can tell, signed in
    /// (Claude API: a key is saved; OpenCode: its service answers).
    pub ready: bool,
    /// Why it isn't ready, for a person.
    pub note: Option<String>,
}

/// Every backend, in `AssistantBackend` order.
pub const BACKENDS: [AssistantBackend; 4] = [
    AssistantBackend::ClaudeCode,
    AssistantBackend::Codex,
    AssistantBackend::OpenCode,
    AssistantBackend::ClaudeApi,
];

/// How long `--version` and a sign-in check may take.
const QUICK: Duration = Duration::from_secs(10);

/// Every backend, found or not, in `AssistantBackend` order. Blocks for a
/// few seconds at most (the first call asks the login shell); call it off
/// the UI thread.
pub fn detect() -> Vec<Found> {
    // Asked once here, so the threads below share the answer.
    search::shell();
    std::thread::scope(|scope| {
        let found: Vec<_> = BACKENDS
            .map(|backend| scope.spawn(move || find(backend)))
            .into_iter()
            .collect();
        found
            .into_iter()
            .zip(BACKENDS)
            .map(|(handle, backend)| {
                handle.join().unwrap_or_else(|_| Found {
                    backend,
                    path: None,
                    version: None,
                    ready: false,
                    note: Some("Hex couldn't check this agent".into()),
                })
            })
            .collect()
    })
}

/// A backend's display name.
pub fn name(backend: AssistantBackend) -> &'static str {
    match backend {
        AssistantBackend::ClaudeCode => "Claude Code",
        AssistantBackend::Codex => "Codex",
        AssistantBackend::OpenCode => "OpenCode",
        AssistantBackend::ClaudeApi => "Claude API",
    }
}

fn program(backend: AssistantBackend) -> Option<&'static str> {
    match backend {
        AssistantBackend::ClaudeCode => Some("claude"),
        AssistantBackend::Codex => Some("codex"),
        AssistantBackend::OpenCode => Some("opencode"),
        AssistantBackend::ClaudeApi => None,
    }
}

/// The executable for `backend`, if installed: a stable path (never a
/// shell's temporary fnm directory). The first call may ask the login
/// shell (a few seconds at most).
pub fn executable(backend: AssistantBackend) -> Option<PathBuf> {
    search::find(program(backend)?)
}

fn find(backend: AssistantBackend) -> Found {
    let mut found = Found {
        backend,
        path: None,
        version: None,
        ready: false,
        note: None,
    };
    if backend == AssistantBackend::ClaudeApi {
        found.ready = crate::assistant::credentials::has_api_key();
        if !found.ready {
            found.note = Some("Add an Anthropic API key".into());
        }
        return found;
    }
    found.path = executable(backend);
    let Some(path) = found.path.clone() else {
        found.note = Some(format!("{} isn't installed", name(backend)));
        return found;
    };
    match version_of(&path) {
        Ok(version) => found.version = version,
        Err(why) => {
            found.note = Some(format!("{} doesn't start from Hex: {why}", name(backend)));
            return found;
        }
    }
    let (ready, note) = match backend {
        AssistantBackend::ClaudeCode => claude_signed_in(&path),
        AssistantBackend::Codex => codex_signed_in(&path),
        AssistantBackend::OpenCode => opencode_signed_in(&path),
        AssistantBackend::ClaudeApi => unreachable!("handled above"),
    };
    found.ready = ready;
    found.note = note;
    found
}

/// OpenCode is ready with a saved key for the chosen provider (any
/// provider's while none is chosen), or with a provider of its own: one it
/// lists models for (`opencode models --standalone`), its free `opencode`
/// models aside. Tasks run it standalone, so its service needn't run.
fn opencode_signed_in(path: &Path) -> (bool, Option<String>) {
    let chosen = crate::app_settings::assistant_runtime()
        .and_then(|(_, settings)| settings.opencode_provider);
    if providers::byok_ready(chosen.as_deref()) {
        return (true, None);
    }
    let mut command = agent_command(path);
    command.args(["models", "--standalone"]);
    let own = run::bounded(command, QUICK)
        .map(|ran| providers::signed_in_providers(&ran.stdout))
        .unwrap_or_default();
    opencode_readiness(chosen.as_deref(), &own)
}

/// What OpenCode's readiness is without a saved key, given the providers it
/// has its own credentials for.
pub fn opencode_readiness(chosen: Option<&str>, own: &[String]) -> (bool, Option<String>) {
    match chosen.and_then(providers::provider) {
        Some(provider) if own.iter().any(|id| id == provider.id) => (true, None),
        Some(provider) => (
            false,
            Some(format!("Add your {} API key for OpenCode", provider.name)),
        ),
        None if !own.is_empty() => (true, None),
        None => (
            false,
            Some(
                "Add an API key (DeepSeek, OpenRouter, …) or sign in to a provider in OpenCode"
                    .into(),
            ),
        ),
    }
}

/// The version `path --version` prints, or why it didn't run.
pub(in crate::assistant) fn version_of(path: &Path) -> std::result::Result<Option<String>, String> {
    let mut command = agent_command(path);
    command.arg("--version");
    let ran = run::bounded(command, QUICK).map_err(|error| error.to_string())?;
    if ran.timed_out() {
        return Err("`--version` took too long".into());
    }
    if !ran.success() {
        return Err(ran.last_words());
    }
    Ok(parse_version(&ran.stdout).or_else(|| parse_version(&ran.stderr)))
}

/// The first version number in `text`: `2.1.318 (Claude Code)`,
/// `codex-cli 0.166.0`, `opencode v2.2.6`, `agent-desktop 1.2.0`.
pub fn parse_version(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .map(|word| word.trim_start_matches(['v', 'V']))
        .find(|word| {
            let mut parts = word.split('.');
            let numeric = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
            parts.next().is_some_and(numeric)
                && parts
                    .next()
                    .is_some_and(|part| numeric(part.split(['-', '+']).next().unwrap_or_default()))
        })
        .map(str::to_owned)
}

/// `claude auth status` (JSON with `loggedIn`).
fn claude_signed_in(path: &Path) -> (bool, Option<String>) {
    let mut command = agent_command(path);
    command.args(["auth", "status"]);
    let sign_in = format!(
        "Run `claude` once in {} to sign in",
        os::Os::HERE.terminal()
    );
    match run::bounded(command, QUICK) {
        Ok(ran) => match claude_logged_in(&ran.stdout) {
            Some(true) => (true, None),
            Some(false) => (false, Some(sign_in)),
            // An older Claude Code without `auth status`: the first task
            // tells.
            None => (true, None),
        },
        Err(error) => (false, Some(format!("Claude Code didn't start: {error}"))),
    }
}

/// `loggedIn` in `claude auth status`' JSON.
pub fn claude_logged_in(output: &str) -> Option<bool> {
    let start = output.find('{')?;
    let value: serde_json::Value = serde_json::from_str(output[start..].trim()).ok()?;
    value.get("loggedIn")?.as_bool()
}

/// `codex login status`: "Logged in using ChatGPT" (or an API key).
fn codex_signed_in(path: &Path) -> (bool, Option<String>) {
    let mut command = agent_command(path);
    command.args(["login", "status"]);
    let sign_in = format!(
        "Run `codex login` once in {} to sign in",
        os::Os::HERE.terminal()
    );
    match run::bounded(command, QUICK) {
        Ok(ran) if ran.success() && codex_logged_in(&format!("{}\n{}", ran.stdout, ran.stderr)) => {
            (true, None)
        }
        Ok(_) => (false, Some(sign_in)),
        Err(error) => (false, Some(format!("Codex didn't start: {error}"))),
    }
}

pub fn codex_logged_in(output: &str) -> bool {
    output.lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("logged in")
    })
}

/// The environment for an agent Hex starts (and for the agent-desktop it
/// starts): Hex's own, less another agent's session markers, with a PATH
/// that has the agents, `node`, and agent-desktop. Use it whole:
/// `command.env_clear().envs(agent_env())`, or [`agent_command`].
pub fn agent_env() -> Vec<(String, String)> {
    let extra: Vec<PathBuf> = ["claude", "codex", "opencode", "agent-desktop", "node"]
        .into_iter()
        .filter_map(search::find)
        .collect();
    let path = search::path_value(&extra);
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| key != "PATH" && !is_session_marker(key))
        .collect();
    env.push(("PATH".into(), path));
    env
}

/// What a parent agent sets for its own children; a task Hex starts is not
/// one of them (Claude Code refuses to start "inside" another session).
fn is_session_marker(key: &str) -> bool {
    matches!(
        key,
        "CLAUDECODE"
            | "CLAUDE_PID"
            | "CLAUDE_EFFORT"
            | "CLAUDE_AGENT_SDK_VERSION"
            | "CLAUDE_CODE_ENTRYPOINT"
            | "CLAUDE_CODE_SESSION_ID"
            | "CLAUDE_CODE_CHILD_SESSION"
            | "CLAUDE_CODE_SESSION_ATTENDED"
            | "CLAUDE_CODE_MESSAGING_SOCKET"
            | "CLAUDE_CODE_MESSAGING_TOKEN"
            | "CLAUDE_CODE_EXECPATH"
            | "CODEX_THREAD_ID"
            | "CODEX_SANDBOX"
            | "CODEX_SANDBOX_NETWORK_DISABLED"
            | "AGENT_DESKTOP_SESSION"
            | "AGENT_DESKTOP_AGENT_ID"
    )
}

/// A command for `program` with [`agent_env`], that never shows a window
/// (on Windows, an npm shim is started through what it runs: `launch`).
pub fn agent_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = launch::command(Path::new(program.as_ref()));
    command.env_clear().envs(agent_env());
    command
}

/// Forgets what was found (the login shell's PATH), so the next call looks
/// again: after an install, or when setup opens.
pub fn refresh() {
    search::forget();
}

/// agent-desktop as found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentDesktop {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub accessibility: bool,
    pub screen_recording: bool,
}

/// agent-desktop's path, version, and permissions as it reports them when
/// Hex starts it (so they are Hex's grants). Blocks briefly; off the UI
/// thread.
pub fn agent_desktop() -> AgentDesktop {
    let Some(path) = search::find("agent-desktop") else {
        return AgentDesktop::default();
    };
    let version = version_of(&path).ok().flatten();
    let grants = agent_desktop_grants(&path).unwrap_or_default();
    AgentDesktop {
        path: Some(path),
        version,
        accessibility: grants.accessibility,
        screen_recording: grants.screen_recording,
    }
}

fn agent_desktop_grants(path: &Path) -> std::result::Result<desktop::Grants, String> {
    agent_desktop_report(path).map(|(grants, _)| grants)
}

/// What agent-desktop asks of this OS: `permissions` on macOS, where its
/// grants are Accessibility and Screen Recording; `status` elsewhere, which
/// says whether UI Automation (Windows) or the accessibility bus (Linux)
/// answers.
pub fn desktop_report_command(os: os::Os) -> &'static str {
    if os == os::Os::Mac {
        "permissions"
    } else {
        "status"
    }
}

/// agent-desktop's grants as it reports them when Hex starts it, and what
/// it suggests for a missing one.
pub(in crate::assistant) fn agent_desktop_report(
    path: &Path,
) -> std::result::Result<(desktop::Grants, Option<String>), String> {
    match desktop_call(path, &[desktop_report_command(os::Os::HERE)])? {
        desktop::Reply::Ok(data) => desktop::grants(&data)
            .map(|grants| (grants, desktop::suggestion(&data)))
            .ok_or_else(|| "agent-desktop didn't say its permissions".into()),
        desktop::Reply::Failed { message, .. } => Err(message),
    }
}

/// Runs one read-only agent-desktop command from Hex and reads its reply.
pub(in crate::assistant) fn desktop_call(
    path: &Path,
    args: &[&str],
) -> std::result::Result<desktop::Reply, String> {
    let mut command = agent_command(path);
    command.args(args);
    let ran = run::bounded(command, Duration::from_secs(15)).map_err(|error| error.to_string())?;
    if ran.timed_out() {
        return Err(format!("`agent-desktop {}` took too long", args.join(" ")));
    }
    desktop::parse(&ran.stdout).ok_or_else(|| {
        let words = ran.last_words();
        if words.is_empty() {
            "agent-desktop said nothing".into()
        } else {
            words
        }
    })
}

/// Opens System Settings at agent-desktop's missing permission
/// (`agent-desktop permissions --guide`), with its guide showing Hex as the
/// app to switch on. Returns at once. macOS only: Windows and Linux have
/// nothing to grant.
pub fn guide_permissions() {
    if os::Os::HERE != os::Os::Mac {
        // Only macOS asks for grants; elsewhere setup never offers this.
        tracing::debug!("agent-desktop has no permission guide on this OS");
        return;
    }
    let Some(path) = search::find("agent-desktop") else {
        tracing::warn!("agent-desktop isn't installed; nothing to guide");
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("hex-assistant-guide".into())
        .spawn(
            move || match desktop_call(&path, &["permissions", "--guide"]) {
                Ok(desktop::Reply::Ok(data)) => {
                    tracing::info!(guide = %data, "agent-desktop's permission guide");
                }
                Ok(desktop::Reply::Failed { code, message }) => {
                    tracing::warn!(code, message, "agent-desktop's permission guide failed");
                }
                Err(error) => tracing::warn!(error, "agent-desktop's permission guide failed"),
            },
        );
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start agent-desktop's permission guide");
    }
}

/// agent-desktop as an agent's MCP server (`agent-desktop mcp`, plus
/// `args` such as `--session <id> --agent-id <id>`), with the PATH Hex gives
/// agents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpServer {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub path_env: String,
}

/// The server's name in each agent's config; its tools are then
/// `mcp__agent-desktop__desktop_*` (Claude Code), `desktop_*` (Codex), and
/// `agent-desktop_desktop_*` (OpenCode).
pub const MCP_NAME: &str = "agent-desktop";

/// agent-desktop as an MCP server with `args`; on Windows, an npm shim's
/// `node.exe` and script, which an agent's MCP client can start.
pub fn mcp_server(args: &[&str]) -> Option<McpServer> {
    let found = search::find("agent-desktop")?;
    let launch = launch::here(&found);
    let extra = [found, launch.program.clone()];
    Some(McpServer {
        command: launch.program,
        args: launch
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .chain(
                std::iter::once("mcp")
                    .chain(args.iter().copied())
                    .map(str::to_owned),
            )
            .collect(),
        path_env: search::path_value(&extra),
    })
}

impl McpServer {
    /// For `claude --mcp-config <this> --strict-mcp-config`.
    pub fn claude_config(&self) -> String {
        self.claude_config_as(MCP_NAME)
    }

    /// [`Self::claude_config`] with the server under `name` (another
    /// server, such as `browser::MCP_NAME`).
    pub fn claude_config_as(&self, name: &str) -> String {
        serde_json::json!({ "mcpServers": { name: self.claude_entry() } }).to_string()
    }

    /// The server's entry in a Claude Code `mcpServers` map, for a config
    /// with several servers.
    pub fn claude_entry(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "stdio",
            "command": self.command,
            "args": self.args,
            "env": { "PATH": self.path_env },
        })
    }

    /// `-c` overrides for `codex exec` that add the server, with only
    /// `tools` enabled (all when empty), approved without asking: `codex
    /// exec` can't ask.
    pub fn codex_args(&self, tools: &[&str]) -> Vec<String> {
        self.codex_args_as(MCP_NAME, tools)
    }

    /// [`Self::codex_args`] with the server under `name`.
    pub fn codex_args_as(&self, name: &str, tools: &[&str]) -> Vec<String> {
        let key = format!("mcp_servers.{name}");
        let toml = |value: serde_json::Value| value.to_string();
        let mut args = vec![
            format!(
                "{key}.command={}",
                toml(self.command.to_string_lossy().into())
            ),
            format!("{key}.args={}", toml(self.args.clone().into())),
            format!("{key}.env.PATH={}", toml(self.path_env.clone().into())),
            format!("{key}.default_tools_approval_mode=\"approve\""),
        ];
        if !tools.is_empty() {
            args.push(format!(
                "{key}.enabled_tools={}",
                toml(tools.to_vec().into())
            ));
        }
        args.into_iter()
            .flat_map(|arg| ["-c".to_owned(), arg])
            .collect()
    }

    /// The `mcp` entry of an OpenCode config (`OPENCODE_CONFIG_CONTENT` for
    /// `opencode run --standalone`).
    pub fn opencode_config(&self) -> serde_json::Value {
        self.opencode_config_as(MCP_NAME)
    }

    /// [`Self::opencode_config`] with the server under `name`.
    pub fn opencode_config_as(&self, name: &str) -> serde_json::Value {
        let mut command = vec![self.command.to_string_lossy().into_owned()];
        command.extend(self.args.iter().cloned());
        serde_json::json!({
            name: {
                "type": "local",
                "command": command,
                "environment": { "PATH": self.path_env },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_each_tools_own_words() {
        for (text, version) in [
            ("2.1.318 (Claude Code)\n", "2.1.318"),
            ("codex-cli 0.166.0\n", "0.166.0"),
            ("opencode v2.2.6\n", "2.2.6"),
            ("agent-desktop 1.2.0\n", "1.2.0"),
            ("Homebrew 5.1.0-beta\n", "5.1.0-beta"),
        ] {
            assert_eq!(parse_version(text).as_deref(), Some(version), "{text}");
        }
        assert_eq!(parse_version("no version here 3"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn sign_in_is_read_from_each_agents_status() {
        let signed_in = r#"{
  "loggedIn": true,
  "authMethod": "oauth_token"
}"#;
        assert_eq!(claude_logged_in(signed_in), Some(true));
        assert_eq!(claude_logged_in(r#"{"loggedIn": false}"#), Some(false));
        assert_eq!(claude_logged_in("error: unknown command 'auth'"), None);
        assert!(codex_logged_in("Logged in using ChatGPT\n"));
        assert!(codex_logged_in(
            "WARNING: proxy\nLogged in using an API key - sk-…\n"
        ));
        assert!(!codex_logged_in("Not logged in\n"));
    }

    #[test]
    fn opencode_is_ready_with_a_provider_of_its_own() {
        let own = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        // Its own sign-in to any provider (this Mac: DeepSeek and OpenAI).
        assert_eq!(
            opencode_readiness(None, &own(&["deepseek", "openai"])),
            (true, None)
        );
        // Only the free `opencode` models (already left out): not ready.
        let (ready, note) = opencode_readiness(None, &[]);
        assert!(!ready);
        assert!(note.unwrap().starts_with("Add an API key"));
        // A chosen provider must be one it has credentials for.
        assert_eq!(
            opencode_readiness(Some("deepseek"), &own(&["deepseek"])),
            (true, None)
        );
        assert_eq!(
            opencode_readiness(Some("groq"), &own(&["deepseek"])),
            (false, Some("Add your Groq API key for OpenCode".into()))
        );
    }

    #[test]
    fn another_agents_session_markers_are_not_passed_on() {
        assert!(is_session_marker("CLAUDECODE"));
        assert!(is_session_marker("CLAUDE_CODE_SESSION_ID"));
        assert!(!is_session_marker("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!is_session_marker("ANTHROPIC_API_KEY"));
        assert!(!is_session_marker("HOME"));
    }

    #[test]
    fn each_agent_gets_agent_desktop_in_its_own_config_format() {
        let server = McpServer {
            command: PathBuf::from("/opt/homebrew/bin/agent-desktop"),
            args: vec!["mcp".into(), "--session".into(), "s1".into()],
            path_env: "/opt/homebrew/bin:/usr/bin".into(),
        };
        let claude: serde_json::Value = serde_json::from_str(&server.claude_config()).unwrap();
        assert_eq!(
            claude["mcpServers"]["agent-desktop"]["command"],
            "/opt/homebrew/bin/agent-desktop"
        );
        assert_eq!(claude["mcpServers"]["agent-desktop"]["args"][2], "s1");
        assert_eq!(
            server.codex_args(&["desktop_status"]),
            [
                "-c",
                r#"mcp_servers.agent-desktop.command="/opt/homebrew/bin/agent-desktop""#,
                "-c",
                r#"mcp_servers.agent-desktop.args=["mcp","--session","s1"]"#,
                "-c",
                r#"mcp_servers.agent-desktop.env.PATH="/opt/homebrew/bin:/usr/bin""#,
                "-c",
                r#"mcp_servers.agent-desktop.default_tools_approval_mode="approve""#,
                "-c",
                r#"mcp_servers.agent-desktop.enabled_tools=["desktop_status"]"#,
            ]
        );
        assert_eq!(
            server.opencode_config()["agent-desktop"]["command"],
            serde_json::json!(["/opt/homebrew/bin/agent-desktop", "mcp", "--session", "s1"])
        );
    }
}
