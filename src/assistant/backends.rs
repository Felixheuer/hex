//! Finding the agents installed on this Mac (Claude Code, Codex, OpenCode)
//! and Cua Driver, the desktop driver behind every task's guard. A GUI app does not
//! inherit the person's shell PATH, so this looks where installers put them
//! and asks the login shell (`search`), and gives every agent it starts that
//! PATH (`agent_env`), so a Node-based agent finds `node`.
//!
//! Setup proves each piece works with `check`'s self-tests, run from Agent-Desk so
//! macOS judges Agent-Desk's own permissions: an agent Agent-Desk starts, and the
//! Cua Driver it starts, act with Agent-Desk.app's Accessibility and Screen
//! Recording, not Terminal's.
//!
//! OWNER: hands.

mod check;
pub(in crate::assistant) mod desktop;
#[cfg(debug_assertions)]
pub mod doctor;
mod install;
mod keys;
mod providers;
mod recommend;
pub(in crate::assistant) mod run;
pub(in crate::assistant) mod search;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};

use crate::app_settings::AssistantBackend;

pub use check::{
    Check, CheckResult, CheckState, Fix, opencode_recent_model, self_test, self_tests,
};
pub use install::{
    Installers, Os, Plan, Step, Tool, apply_fix, apply_fix_with, install_command, plan,
};
pub use providers::{
    Provider, agent_env_for, has_provider_key, merge_config, opencode_model,
    opencode_provider_config, parse_models, provider, providers, set_provider_key,
    signed_in_providers, verify_provider_key,
};
pub use recommend::{Recommendation, recommendations};

/// One agent found on this computer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Found {
    pub backend: AssistantBackend,
    /// The executable (`None` for `ClaudeApi`, and for OpenCode when only its
    /// service is running).
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Ready to take a task: installed and, as far as Agent-Desk can tell, signed in
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
                    note: Some("Agent-Desk couldn't check this agent".into()),
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
            found.note = Some(format!(
                "{} doesn't start from Agent-Desk: {why}",
                name(backend)
            ));
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
/// `codex-cli 0.166.0`, `opencode v2.2.6`, `cua-driver 0.49.3`.
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
    let sign_in = if cfg!(target_os = "macos") {
        "Run `claude` once in Terminal to sign in"
    } else {
        "Run `claude` once in a terminal to sign in"
    };
    match run::bounded(command, QUICK) {
        Ok(ran) => match claude_logged_in(&ran.stdout) {
            Some(true) => (true, None),
            Some(false) => (false, Some(sign_in.into())),
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
    let sign_in = if cfg!(target_os = "macos") {
        "Run `codex login` once in Terminal to sign in"
    } else {
        "Run `codex login` once in a terminal to sign in"
    };
    match run::bounded(command, QUICK) {
        Ok(ran) if ran.success() && codex_logged_in(&format!("{}\n{}", ran.stdout, ran.stderr)) => {
            (true, None)
        }
        Ok(_) => (false, Some(sign_in.into())),
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

/// The environment for an agent Agent-Desk starts: Agent-Desk's own, less another
/// agent's session markers, with a PATH that has the agents and `node`. Use it
/// whole:
/// `command.env_clear().envs(agent_env())`, or [`agent_command`].
pub fn agent_env() -> Vec<(String, String)> {
    let extra: Vec<PathBuf> = ["claude", "codex", "opencode", "node"]
        .into_iter()
        .filter_map(search::find)
        .collect();
    let path = search::path_value(&extra);
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| !key.eq_ignore_ascii_case("PATH") && !is_session_marker(key))
        .collect();
    env.push(("PATH".into(), path));
    #[cfg(windows)]
    for (key, fallback) in WINDOWS_DEFAULTS {
        if !env.iter().any(|(have, _)| have.eq_ignore_ascii_case(key)) {
            env.push(((*key).into(), (*fallback).into()));
        }
    }
    env
}

/// What a Windows program can't start without (Node needs `SystemRoot`,
/// npm's shims `ComSpec` and `PATHEXT`), with a default for each in case
/// Agent-Desk was started without them.
#[cfg(windows)]
const WINDOWS_DEFAULTS: &[(&str, &str)] = &[
    ("SystemRoot", r"C:\Windows"),
    ("ComSpec", r"C:\Windows\System32\cmd.exe"),
    ("PATHEXT", ".COM;.EXE;.BAT;.CMD"),
];

/// Besides `PATH`, the variables of Agent-Desk's own an MCP server an agent starts
/// gets: on Windows the ones a program needs to start and find the
/// person's folders (agents pass a server only a few of their own).
pub fn mcp_env_extra() -> Vec<(String, String)> {
    #[cfg(windows)]
    {
        const KEEP: &[&str] = &[
            "SystemRoot",
            "SystemDrive",
            "windir",
            "PATHEXT",
            "ComSpec",
            "APPDATA",
            "LOCALAPPDATA",
            "USERPROFILE",
            "USERNAME",
            "HOMEDRIVE",
            "HOMEPATH",
            "TEMP",
            "TMP",
            "ProgramData",
            "ProgramFiles",
        ];
        KEEP.iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| ((*key).to_owned(), value))
            })
            .collect()
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// What a parent agent sets for its own children; a task Agent-Desk starts is not
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
    )
}

/// A command for `program` with [`agent_env`], that never shows a window.
/// A Windows npm shim runs as what it wraps ([`search::launch`]).
pub fn agent_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = search::command(Path::new(program.as_ref()));
    command.env_clear().envs(agent_env());
    command
}

/// Forgets what was found (the login shell's PATH), so the next call looks
/// again: after an install, or when setup opens.
pub fn refresh() {
    search::forget();
}

/// Cua Driver, the desktop driver, as found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Driver {
    pub path: Option<PathBuf>,
    /// The version tasks run.
    pub version: Option<String>,
    /// Agent-Desk's grants, which the driver acts with (it runs in Agent-Desk's
    /// process tree).
    pub accessibility: bool,
    pub screen_recording: bool,
    /// How its last update check went (`driver::update::Status::line`):
    /// "Cua Driver 0.49.3 · up to date".
    pub update: String,
}

/// Cua Driver's path and version, Agent-Desk's grants, and its update
/// status. Quick; off the UI thread all the same.
pub fn driver() -> Driver {
    let path = crate::assistant::driver::find();
    let permissions = crate::assistant::computer::permissions_now();
    Driver {
        version: path
            .as_ref()
            .and_then(|_| crate::assistant::driver::cua::version()),
        path,
        accessibility: permissions.accessibility,
        screen_recording: permissions.screen_recording,
        update: crate::assistant::driver::update::status().line(),
    }
}

/// An agent's MCP server (the guard's, a browser's): its command and
/// arguments, with the PATH Agent-Desk gives agents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpServer {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub path_env: String,
}

/// The server's name in each agent's config; its tools are then
/// `mcp__desktop__desktop_*` (Claude Code), `desktop_*` (Codex), and
/// `desktop_desktop_*` (OpenCode), served by the guard from Cua Driver
/// (`assistant::driver`).
pub const MCP_NAME: &str = "desktop";

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
        let (command, args) = self.launch();
        serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": args,
            "env": self.env(),
        })
    }

    /// The program and arguments that start the server: a Windows npm shim
    /// as the program it wraps ([`search::launch`]), since agents start
    /// servers without a shell.
    pub fn launch(&self) -> (String, Vec<String>) {
        let (program, lead) = search::launch(&self.command);
        let args = lead
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .collect();
        (program.to_string_lossy().into_owned(), args)
    }

    /// The server's environment: its `PATH`, and on Windows what a program
    /// needs to start ([`mcp_env_extra`]).
    pub fn env(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut env = serde_json::Map::new();
        env.insert("PATH".into(), self.path_env.clone().into());
        for (key, value) in mcp_env_extra() {
            env.insert(key, value.into());
        }
        env
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
        let (command, server_args) = self.launch();
        let mut args = vec![
            format!("{key}.command={}", toml(command.into())),
            format!("{key}.args={}", toml(server_args.into())),
        ];
        for (variable, value) in self.env() {
            args.push(format!("{key}.env.{variable}={}", toml(value)));
        }
        args.push(format!("{key}.default_tools_approval_mode=\"approve\""));
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
        let (program, args) = self.launch();
        let mut command = vec![program];
        command.extend(args);
        serde_json::json!({
            name: {
                "type": "local",
                "command": command,
                "environment": self.env(),
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
            ("cua-driver 0.49.3\n", "0.49.3"),
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
    fn each_agent_gets_the_guard_in_its_own_config_format() {
        let server = McpServer {
            command: PathBuf::from("/opt/homebrew/bin/agent-desk"),
            args: vec!["assistant".into(), "mcp".into(), "s1".into()],
            path_env: "/opt/homebrew/bin:/usr/bin".into(),
        };
        let claude: serde_json::Value = serde_json::from_str(&server.claude_config()).unwrap();
        assert_eq!(
            claude["mcpServers"]["desktop"]["command"],
            "/opt/homebrew/bin/agent-desk"
        );
        assert_eq!(claude["mcpServers"]["desktop"]["args"][2], "s1");
        assert_eq!(
            server.codex_args(&["desktop_status"]),
            [
                "-c",
                r#"mcp_servers.desktop.command="/opt/homebrew/bin/agent-desk""#,
                "-c",
                r#"mcp_servers.desktop.args=["assistant","mcp","s1"]"#,
                "-c",
                r#"mcp_servers.desktop.env.PATH="/opt/homebrew/bin:/usr/bin""#,
                "-c",
                r#"mcp_servers.desktop.default_tools_approval_mode="approve""#,
                "-c",
                r#"mcp_servers.desktop.enabled_tools=["desktop_status"]"#,
            ]
        );
        assert_eq!(
            server.opencode_config()["desktop"]["command"],
            serde_json::json!(["/opt/homebrew/bin/agent-desk", "assistant", "mcp", "s1"])
        );
    }
}
