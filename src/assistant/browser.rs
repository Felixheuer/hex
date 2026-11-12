//! Hidden browsers for web work: agent-browser (Vercel Labs), one session
//! per task, so twenty web tasks run at once without a window on the
//! person's screen and without waiting on the desktop's turn. Tasks get it
//! as a second tool beside the guard's desktop tools and are told to prefer
//! it for anything on the web.
//!
//! agent-browser runs headless Chrome for Testing (`agent-browser install`)
//! or, for clean reading, Lightpanda (a light headless browser, about a
//! tenth of Chrome's memory) behind a daemon per session, and has its own
//! MCP server (`agent-browser mcp`). A task gets that server through `hex
//! assistant browser-mcp --task <id>` ([`proxy`]), which pins every call to
//! one of the task's own browsers ([`sessions`]: its main one and up to
//! seven named ones) in Hex's own namespace and withholds the arguments that
//! would show a window, reach another task, or persist state.
//!
//! **Sign-ins.** A hidden browser has none of the person's cookies, and Hex
//! never copies them from Chrome or Safari. Instead the person may sign in
//! once in a visible window of Hex's own ([`sign_in`]); agent-browser keeps
//! that window's cookies and storage under a restore key, and a task's
//! browser that asks for it (`signedIn`) starts in Chrome from a read-only
//! copy of them (it never saves back).
//!
//! OWNER: browser.

mod proxy;
pub mod sessions;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

use crate::assistant::backends::{self, McpServer, run, search};

pub use proxy::serve;
pub use sessions::{Engine, MAX_SESSIONS};

/// The server's name in each agent's config: its tools are then
/// `mcp__agent-browser__agent_browser_*` (Claude Code), `agent_browser_*`
/// (Codex), and `agent-browser_agent_browser_*` (OpenCode).
pub const MCP_NAME: &str = "agent-browser";

/// Hex's own agent-browser namespace: its sessions and saved state stay
/// apart from the person's own agent-browser use.
const NAMESPACE: &str = "hex";

/// The sign-in window's session, and the restore key every task loads.
const SIGN_IN: &str = "hex-sign-in";

/// A task's browser that saw no command for this long closes itself, in
/// case the task's end never closed it.
const IDLE_MS: &str = "1800000";

/// How long one agent-browser command may take (a cold Chrome launch is a
/// few seconds).
const COMMAND_LIMIT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentBrowser {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Its own Chrome is downloaded (`agent-browser install`).
    pub browser_ready: bool,
}

/// agent-browser's path, version, and whether it has a browser to run.
/// Blocks briefly (two quick commands); off the UI thread.
pub fn agent_browser() -> AgentBrowser {
    let Some(path) = executable() else {
        return AgentBrowser::default();
    };
    let version = backends::version_of(&path).ok().flatten();
    let browser_ready = browser_installed(&path).unwrap_or(false);
    AgentBrowser {
        path: Some(path),
        version,
        browser_ready,
    }
}

/// The agent-browser executable, if installed.
pub fn executable() -> Option<PathBuf> {
    search::find("agent-browser")
}

/// `agent-browser doctor --offline --quick --json`: whether a Chrome it can
/// launch is there.
fn browser_installed(path: &Path) -> Option<bool> {
    let mut command = base_command(path);
    command.args(["doctor", "--offline", "--quick", "--json"]);
    let ran = run::bounded(command, Duration::from_secs(15)).ok()?;
    chrome_installed(&ran.stdout)
}

/// `chrome.installed` in `doctor --json`'s checks.
pub fn chrome_installed(doctor: &str) -> Option<bool> {
    let report = last_json(doctor)?;
    let check = report
        .get("checks")?
        .as_array()?
        .iter()
        .find(|check| check.get("id").and_then(Value::as_str) == Some("chrome.installed"))?;
    Some(check.get("status").and_then(Value::as_str) == Some("pass"))
}

/// `brew install agent-browser` (else npm), then `agent-browser install`
/// (its Chrome); blocks. What isn't missing isn't installed again.
pub fn install() -> Result<()> {
    install_with(|_| {})
}

/// [`install`], handing `progress` each line of the installers' output.
pub fn install_with(progress: impl FnMut(&str)) -> Result<()> {
    backends::apply_fix_with(backends::Fix::InstallAgentBrowser, progress)
}

/// Lightpanda, if installed (`~/.local/bin/lightpanda` from its installer).
pub fn lightpanda() -> Option<PathBuf> {
    search::find("lightpanda")
}

/// The engine behind a task's clean browsers: Lightpanda when it's
/// installed, else Chrome.
pub fn clean_engine() -> Engine {
    if lightpanda().is_some() {
        Engine::Lightpanda
    } else {
        Engine::Chrome
    }
}

/// Installs Lightpanda with its official script (into `~/.local/bin`);
/// blocks. Nothing happens when it's installed.
pub fn install_lightpanda_with(progress: impl FnMut(&str)) -> Result<()> {
    backends::apply_fix_with(backends::Fix::InstallLightpanda, progress)
}

/// The browser tool for `task`: its own browsers, closed when the task
/// ends ([`close`]). `None` when agent-browser isn't installed, or while
/// `background_browser` is off.
pub fn mcp_server(task: &str) -> Option<McpServer> {
    let enabled = crate::app_settings::assistant_runtime()
        .is_none_or(|(_, settings)| settings.background_browser);
    if !enabled {
        return None;
    }
    let browser = executable()?;
    let hex = std::env::current_exe().ok()?;
    Some(server(hex, task, &browser))
}

fn server(hex: PathBuf, task: &str, browser: &Path) -> McpServer {
    McpServer {
        command: hex,
        args: ["assistant", "browser-mcp", "--task", task]
            .map(str::to_owned)
            .to_vec(),
        path_env: search::path_value(&[browser.to_owned()]),
    }
}

/// Closes every browser of `task` (nothing is saved). Blocks for a moment;
/// quick when it never opened one.
pub fn close(task: &str) {
    let Some(path) = executable() else {
        return;
    };
    let mut open = running(&path, task);
    let main = session(task);
    if !open.contains(&main) {
        // Listed or not, the main one is closed (a quick no-op if absent).
        open.push(main);
    }
    std::thread::scope(|scope| {
        for session in &open {
            let path = &path;
            scope.spawn(move || match call(path, session, &["close"]) {
                Ok(_) => tracing::debug!(session, "closed a task's browser"),
                Err(error) => tracing::warn!(session, error, "could not close a task's browser"),
            });
        }
    });
}

/// `task`'s browsers that agent-browser has open.
pub(crate) fn running(path: &Path, task: &str) -> Vec<String> {
    let listed = call(path, &session(task), &["session", "list"]).unwrap_or_default();
    listed
        .get("sessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|session| sessions::belongs(task, session))
        .map(str::to_owned)
        .collect()
}

/// `task`'s session name: agent-browser takes letters, digits, `-`, `_`.
pub fn session(task: &str) -> String {
    let id: String = task
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    format!("hex-{id}")
}

/// Opens Hex's own browser in a visible window at `url`, so the person can
/// sign in to the sites tasks should use. Its cookies and storage are saved
/// as they change and when [`finish_sign_in`] closes it; every task's
/// browser starts with them from then on. Returns once the window is open.
pub fn sign_in(url: Option<&str>) -> Result<()> {
    let path = executable().ok_or_else(|| eyre!("agent-browser isn't installed"))?;
    call(
        &path,
        SIGN_IN,
        &["--headed", "open", url.unwrap_or("about:blank")],
    )
    .map(|_| ())
    .map_err(|error| eyre!("Hex's browser didn't open: {error}"))
}

/// Saves and closes the sign-in window.
pub fn finish_sign_in() -> Result<()> {
    let path = executable().ok_or_else(|| eyre!("agent-browser isn't installed"))?;
    call(&path, SIGN_IN, &["close"])
        .map(|_| ())
        .map_err(|error| eyre!("Hex's browser didn't close: {error}"))?;
    private_state();
    Ok(())
}

/// Whether the person signed in to anything in Hex's browser.
pub fn signed_in() -> bool {
    std::fs::read_dir(state_dir().unwrap_or_default()).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| is_sign_in_state(&entry.file_name().to_string_lossy()))
    })
}

/// Forgets every sign-in: the next tasks start signed out.
pub fn forget_sign_in() -> Result<()> {
    let Some(dir) = state_dir() else {
        return Ok(());
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if is_sign_in_state(&entry.file_name().to_string_lossy()) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// agent-browser's saved state for Hex's namespace:
/// `~/.agent-browser/namespaces/hex/state/sessions/<key>-<session>.json`.
fn state_dir() -> Option<PathBuf> {
    Some(
        crate::app_paths::home_dir()?
            .join(".agent-browser/namespaces")
            .join(NAMESPACE)
            .join("state/sessions"),
    )
}

fn is_sign_in_state(file: &str) -> bool {
    file.starts_with(&format!("{SIGN_IN}-")) && file.ends_with(".json")
}

/// The saved sign-ins are session tokens in plain JSON: only this user may
/// read the folder.
fn private_state() {
    #[cfg(unix)]
    if let Some(dir) = state_dir() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
}

/// agent-browser with Hex's agent PATH (and Lightpanda's folder) and none
/// of the person's own `AGENT_BROWSER_*` settings.
fn base_command(path: &Path) -> Command {
    let mut command = crate::desktop::background_process::command(path);
    let extra: Vec<PathBuf> = [Some(path.to_owned()), lightpanda()]
        .into_iter()
        .flatten()
        .collect();
    command.env_clear().envs(
        backends::agent_env()
            .into_iter()
            .filter(|(key, _)| key != "PATH" && !key.starts_with("AGENT_BROWSER_")),
    );
    command.env("PATH", search::path_value(&extra));
    command
}

/// agent-browser for `session` in Hex's namespace, headless and clean; a
/// signed-in browser asks for the sign-ins per call ([`sessions`]). The
/// sign-in window's own session loads and saves them.
pub(crate) fn command(path: &Path, session: &str) -> Command {
    let mut command = base_command(path);
    command.envs(session_env(session, config().as_deref()));
    command
}

/// The environment that pins agent-browser to `session`.
pub fn session_env(session: &str, config: Option<&Path>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = [
        ("AGENT_BROWSER_NAMESPACE", NAMESPACE),
        ("AGENT_BROWSER_SESSION", session),
        ("AGENT_BROWSER_IDLE_TIMEOUT_MS", IDLE_MS),
        // Saved sign-ins otherwise lapse after 30 days.
        ("AGENT_BROWSER_STATE_EXPIRE_DAYS", "365"),
        // Lightpanda says it is an old Chrome, which agent-browser would
        // otherwise warn about in every reply.
        ("AGENT_BROWSER_NO_VERSION_CHECK", "1"),
        ("LIGHTPANDA_DISABLE_TELEMETRY", "true"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    if session == SIGN_IN {
        env.push(("AGENT_BROWSER_RESTORE".into(), SIGN_IN.into()));
        env.push(("AGENT_BROWSER_RESTORE_SAVE".into(), "always".into()));
    }
    if let Some(config) = config {
        env.push((
            "AGENT_BROWSER_CONFIG".into(),
            config.to_string_lossy().into_owned(),
        ));
    }
    env
}

/// Hex's agent-browser config, in place of the person's
/// `~/.agent-browser/config.json` (which could make every browser visible
/// or open their own Chrome profile).
fn config() -> Option<PathBuf> {
    let path = crate::assistant::store::assistant_dir()
        .ok()?
        .join("agent-browser.json");
    let wanted = br#"{"headed": false}"#;
    if std::fs::read(&path).ok().as_deref() != Some(wanted.as_slice()) {
        std::fs::write(&path, wanted).ok()?;
    }
    Some(path)
}

/// Runs one agent-browser command for `session` with `--json`, and reads
/// its reply's `data`.
pub(crate) fn call(path: &Path, session: &str, args: &[&str]) -> Result<Value, String> {
    let mut command = command(path, session);
    command.arg("--json").args(args);
    let ran = run::bounded(command, COMMAND_LIMIT).map_err(|error| error.to_string())?;
    if ran.timed_out() {
        return Err(format!(
            "`agent-browser {}` took more than {} s",
            args.join(" "),
            COMMAND_LIMIT.as_secs()
        ));
    }
    reply(&ran.stdout).unwrap_or_else(|| {
        let words = ran.last_words();
        Err(if words.is_empty() {
            "agent-browser said nothing".into()
        } else {
            words
        })
    })
}

/// A `--json` reply: `{"success": true, "data": …}` or `{"success": false,
/// "error": "…"}` (the last line that is one).
pub fn reply(output: &str) -> Option<Result<Value, String>> {
    let value = last_json(output)?;
    match value.get("success")?.as_bool()? {
        true => Some(Ok(value.get("data").cloned().unwrap_or(Value::Null))),
        false => Some(Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("agent-browser failed")
            .to_owned())),
    }
}

fn last_json(output: &str) -> Option<Value> {
    output
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_read_from_the_last_envelope() {
        let opened = r#"{"success":true,"data":{"lifecycle":{"launched":true},"targetId":"F6","title":"Example Domain","url":"https://example.com/"},"error":null}"#;
        let data = reply(&format!("⚠ a warning\n{opened}\n")).unwrap().unwrap();
        assert_eq!(data["title"], "Example Domain");
        let failed = r#"{"success":false,"data":null,"error":"Element not found: #nope."}"#;
        assert_eq!(reply(failed), Some(Err("Element not found: #nope.".into())));
        let invalid = r#"{"error":"Invalid session name 'a b'.","success":false,"type":"invalid_session_name"}"#;
        assert_eq!(
            reply(invalid),
            Some(Err("Invalid session name 'a b'.".into()))
        );
        assert_eq!(reply("✓ Browser closed"), None);
    }

    #[test]
    fn doctor_says_whether_chrome_is_installed() {
        let ready = r#"{"checks":[{"category":"Environment","id":"env.version","message":"CLI version 0.42.0","status":"pass"},{"category":"Chrome","id":"chrome.installed","message":"Google Chrome for Testing 157","status":"pass"}],"success":true}"#;
        let missing = r#"{"checks":[{"category":"Chrome","id":"chrome.installed","message":"Chrome not found","status":"fail","fix":"agent-browser install"}],"success":false}"#;
        assert_eq!(chrome_installed(ready), Some(true));
        assert_eq!(chrome_installed(missing), Some(false));
        assert_eq!(chrome_installed("not json"), None);
    }

    #[test]
    fn each_task_gets_a_valid_session_of_its_own() {
        assert_eq!(
            session("3f2a9c1e-0b7d-4e55-9a10-6c2f8e4d1b00"),
            "hex-3f2a9c1e-0b7d-4e55-9a10-6c2f8e4d1b00"
        );
        assert_eq!(session("a b/c"), "hex-a-b-c");
        assert!(session(&"x".repeat(200)).len() <= 68);
    }

    #[test]
    fn task_browsers_start_clean_and_only_the_sign_in_window_saves() {
        let env = |session| {
            session_env(session, Some(Path::new("/tmp/hex.json")))
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>()
        };
        let task = env("hex-t1");
        assert_eq!(task["AGENT_BROWSER_SESSION"], "hex-t1");
        assert_eq!(task["AGENT_BROWSER_NAMESPACE"], "hex");
        assert!(!task.contains_key("AGENT_BROWSER_RESTORE"));
        assert_eq!(task["LIGHTPANDA_DISABLE_TELEMETRY"], "true");
        assert_eq!(task["AGENT_BROWSER_CONFIG"], "/tmp/hex.json");
        assert_eq!(env(SIGN_IN)["AGENT_BROWSER_RESTORE"], SIGN_IN);
        assert_eq!(env(SIGN_IN)["AGENT_BROWSER_RESTORE_SAVE"], "always");
        assert!(is_sign_in_state("hex-sign-in-hex-sign-in.json"));
        assert!(!is_sign_in_state("other-hex-t1.json"));
    }

    #[test]
    fn the_server_is_hex_itself_for_the_task() {
        let server = server(
            PathBuf::from("/Applications/Hex.app/Contents/MacOS/hex"),
            "t1",
            Path::new("/opt/homebrew/bin/agent-browser"),
        );
        assert_eq!(server.args, ["assistant", "browser-mcp", "--task", "t1"]);
        assert!(
            server
                .path_env
                .split(':')
                .any(|dir| dir == "/opt/homebrew/bin")
        );
        let claude: Value = serde_json::from_str(&server.claude_config_as(MCP_NAME)).unwrap();
        assert_eq!(
            claude["mcpServers"]["agent-browser"]["command"],
            "/Applications/Hex.app/Contents/MacOS/hex"
        );
        assert_eq!(
            server.codex_args_as(MCP_NAME, &["agent_browser_open"])[1],
            r#"mcp_servers.agent-browser.command="/Applications/Hex.app/Contents/MacOS/hex""#
        );
        assert_eq!(
            server.opencode_config_as(MCP_NAME)["agent-browser"]["command"][4],
            "t1"
        );
    }
}
