//! Setup's self-tests: each proves one piece really works from Hex, and
//! says what to do when it doesn't. They only read: the agents are asked to
//! reply "OK" or to call agent-desktop's `desktop_status`, and agent-desktop
//! lists apps and windows and reads the Dock; nothing is clicked, typed, or
//! quit.
//!
//! Run from Hex, so macOS judges Hex.app's permissions: agent-desktop
//! started by Hex (directly, or by an agent Hex started) acts with Hex's
//! Accessibility and Screen Recording.

mod browser;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

use super::{agent_command, desktop, name, run, search};
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::computer::{self, Permission};

/// One self-test.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Check {
    /// Hex can read other apps and the screen (its own Accessibility and
    /// Screen Recording), for a question's context.
    HexPermissions,
    /// The agent answers a tiny prompt ("Reply with OK"), 30 s at most.
    Answers(AssistantBackend),
    /// agent-desktop runs from Hex and has Accessibility and Screen
    /// Recording there.
    DesktopPermissions,
    /// agent-desktop lists the running apps and windows and reads the Dock,
    /// and its window ids lead to windows on screen (for task outlines).
    DesktopRoundTrip,
    /// The agent calls agent-desktop through MCP (`desktop_status`), and
    /// agent-desktop has Accessibility there too. Not for `ClaudeApi`, whose
    /// tasks Hex runs agent-desktop for itself.
    DesktopThroughAgent(AssistantBackend),
    /// agent-browser opens a page in a hidden browser of its own, reads its
    /// title, and closes it.
    BrowserRoundTrip,
    /// The agent opens a page through Hex's browser tool over MCP and reads
    /// its title (for the Claude API, Hex does, as it does for its tasks).
    BrowserThroughAgent(AssistantBackend),
    /// Lightpanda, the light engine for clean (research) browsers, opens a
    /// page through agent-browser. Optional: without it they run Chrome.
    LightBrowser,
}

impl Check {
    /// The checks setup shows for `backend`, in order; the hidden browser's
    /// when `background_browser` is on.
    pub fn for_backend(backend: AssistantBackend, background_browser: bool) -> Vec<Self> {
        let mut checks = vec![
            Self::HexPermissions,
            Self::Answers(backend),
            Self::DesktopPermissions,
            Self::DesktopRoundTrip,
        ];
        if backend != AssistantBackend::ClaudeApi {
            checks.push(Self::DesktopThroughAgent(backend));
        }
        if background_browser {
            checks.push(Self::BrowserRoundTrip);
            // Lightpanda has no Windows build.
            if super::install::Tool::Lightpanda.available_on(super::install::Os::current()) {
                checks.push(Self::LightBrowser);
            }
            checks.push(Self::BrowserThroughAgent(backend));
        }
        checks
    }

    /// What it proves, for its row.
    pub fn title(self) -> String {
        match self {
            Self::HexPermissions => "Hex can read the screen and selected text".into(),
            Self::Answers(backend) => format!("{} answers", name(backend)),
            Self::DesktopPermissions => {
                "agent-desktop has Accessibility and Screen Recording".into()
            }
            Self::DesktopRoundTrip => "agent-desktop reads apps and windows".into(),
            Self::DesktopThroughAgent(backend) => {
                format!("{} can use agent-desktop", name(backend))
            }
            Self::BrowserRoundTrip => "agent-browser opens pages in hidden browsers".into(),
            Self::BrowserThroughAgent(backend) => {
                format!("{} can use agent-browser", name(backend))
            }
            Self::LightBrowser => "Lightpanda reads pages in light browsers".into(),
        }
    }

    /// The checks that must pass first; when one failed, this one fails
    /// with its reason instead of running.
    pub fn needs(self) -> Vec<Self> {
        match self {
            Self::HexPermissions | Self::Answers(_) | Self::DesktopPermissions => Vec::new(),
            Self::DesktopRoundTrip => vec![Self::DesktopPermissions],
            Self::DesktopThroughAgent(backend) => {
                vec![Self::Answers(backend), Self::DesktopPermissions]
            }
            Self::BrowserRoundTrip => Vec::new(),
            Self::LightBrowser => vec![Self::BrowserRoundTrip],
            Self::BrowserThroughAgent(backend) => {
                vec![Self::Answers(backend), Self::BrowserRoundTrip]
            }
        }
    }

    /// How long it may take, at most (plus a few seconds to find things).
    pub fn limit(self) -> Duration {
        match self {
            Self::HexPermissions => Duration::from_secs(10),
            Self::Answers(_) => ANSWER_LIMIT,
            Self::DesktopPermissions | Self::DesktopRoundTrip => Duration::from_secs(45),
            Self::DesktopThroughAgent(_) => MCP_LIMIT,
            Self::BrowserRoundTrip | Self::LightBrowser => browser::ROUND_TRIP_LIMIT,
            Self::BrowserThroughAgent(_) => browser::THROUGH_AGENT_LIMIT,
        }
    }
}

/// A row's state as setup shows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckState {
    Pending,
    Running,
    Ok,
    Failed,
}

/// How a check went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckResult {
    pub check: Check,
    /// `Running` while [`self_tests`] reports one starting; else `Ok` or
    /// `Failed`.
    pub state: CheckState,
    /// What it found, for a person: "Replied “OK” in 2.1 s", "agent-desktop
    /// has no Accessibility".
    pub detail: String,
    /// What would make it pass, when it failed.
    pub fix: Option<Fix>,
    pub took: Duration,
}

impl CheckResult {
    pub fn ok(&self) -> bool {
        self.state == CheckState::Ok
    }

    fn passed(check: Check, detail: impl Into<String>) -> Self {
        Self {
            check,
            state: CheckState::Ok,
            detail: detail.into(),
            fix: None,
            took: Duration::ZERO,
        }
    }

    fn failed(check: Check, detail: impl Into<String>, fix: Option<Fix>) -> Self {
        Self {
            check,
            state: CheckState::Failed,
            detail: detail.into(),
            fix,
            took: Duration::ZERO,
        }
    }
}

/// What setup offers for a failed check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fix {
    /// Opens the agent's install page.
    Install(AssistantBackend),
    /// `brew install agent-desktop` (blocks for a minute or so).
    InstallAgentDesktop,
    /// `brew install agent-browser`, then `agent-browser install` (its
    /// Chrome); a few minutes at most.
    InstallAgentBrowser,
    /// Lightpanda's official installer (into `~/.local/bin`).
    InstallLightpanda,
    /// Opens Terminal on the agent's sign-in (`claude`, `codex login`).
    SignIn(AssistantBackend),
    /// Use a saved API key instead of an account: Codex signs in with the
    /// OpenAI key (`codex login --with-api-key`); OpenCode and the Claude
    /// API only need the key saved (the sheet's key form).
    UseApiKey(AssistantBackend),
    /// `opencode service start`.
    StartOpenCode,
    /// The sheet's API key field: nothing for [`apply_fix`] to do.
    AddApiKey,
    /// agent-desktop's own guide (`permissions --guide`).
    GuidePermissions,
    /// System Settings at Hex's own missing permission.
    OpenSettings(Permission),
    /// Run it again (a timeout, a hiccup).
    Retry,
}

impl Fix {
    /// The fix button's label.
    pub fn label(self) -> String {
        match self {
            Self::Install(backend) => format!("Install {}", name(backend)),
            Self::InstallAgentDesktop => "Install agent-desktop".into(),
            Self::InstallAgentBrowser => "Install agent-browser".into(),
            Self::InstallLightpanda => "Install Lightpanda".into(),
            Self::SignIn(_) => "Sign in".into(),
            Self::UseApiKey(_) => "Use an API key".into(),
            Self::StartOpenCode => "Start OpenCode".into(),
            Self::AddApiKey => "Add API key".into(),
            Self::GuidePermissions => "Grant permissions".into(),
            Self::OpenSettings(permission) => format!("Open {} settings", permission.name()),
            Self::Retry => "Try again".into(),
        }
    }
}

/// Runs `check` (seconds; up to [`Check::limit`]). Off the UI thread.
pub fn self_test(check: Check, settings: &AssistantSettings) -> CheckResult {
    let started = Instant::now();
    let mut result = match check {
        Check::HexPermissions => hex_permissions(check),
        Check::Answers(backend) => answers(check, backend, settings),
        Check::DesktopPermissions => desktop_permissions(check),
        Check::DesktopRoundTrip => round_trip(check),
        Check::DesktopThroughAgent(backend) => through_agent(check, backend, settings),
        Check::BrowserRoundTrip => browser::round_trip(check),
        Check::LightBrowser => browser::light_round_trip(check),
        Check::BrowserThroughAgent(backend) => browser::through_agent(check, backend, settings),
    };
    result.took = started.elapsed();
    tracing::info!(
        check = %check.title(),
        ok = result.ok(),
        detail = %result.detail,
        took_ms = result.took.as_millis() as u64,
        "assistant self-test"
    );
    result
}

/// Runs `checks` in order, reporting each as it starts (`Running`) and as
/// it ends. A check whose [`Check::needs`] failed fails at once with that
/// reason. Blocks until all are done; off the UI thread.
pub fn self_tests(
    checks: &[Check],
    settings: &AssistantSettings,
    mut report: impl FnMut(&CheckResult),
) -> Vec<CheckResult> {
    let mut done: Vec<CheckResult> = Vec::new();
    for &check in checks {
        let unmet = check.needs().into_iter().find_map(|need| {
            done.iter()
                .find(|result| result.check == need && !result.ok())
        });
        let result = match unmet {
            Some(unmet) => CheckResult::failed(
                check,
                format!("Needs “{}” first", unmet.check.title()),
                unmet.fix,
            ),
            None => {
                report(&CheckResult {
                    check,
                    state: CheckState::Running,
                    detail: String::new(),
                    fix: None,
                    took: Duration::ZERO,
                });
                self_test(check, settings)
            }
        };
        report(&result);
        done.push(result);
    }
    done
}

fn hex_permissions(check: Check) -> CheckResult {
    let permissions = computer::permissions_now();
    match (permissions.accessibility, permissions.screen_recording) {
        (true, true) => CheckResult::passed(check, "Accessibility and Screen Recording are on"),
        (false, _) => CheckResult::failed(
            check,
            "Hex needs Accessibility to read the selected text",
            Some(Fix::OpenSettings(Permission::Accessibility)),
        ),
        (true, false) => CheckResult::failed(
            check,
            "Hex needs Screen Recording to send a screenshot with a question",
            Some(Fix::OpenSettings(Permission::ScreenRecording)),
        ),
    }
}

/// Where the agents run during a check: an empty folder of Hex's own, so no
/// project's settings or files come into it.
fn scratch() -> PathBuf {
    let folder = std::env::temp_dir().join("hex-assistant-check");
    if let Err(error) = std::fs::create_dir_all(&folder) {
        tracing::warn!(%error, "could not make the self-test folder");
    }
    folder
}

const ANSWER_LIMIT: Duration = Duration::from_secs(30);
const MCP_LIMIT: Duration = Duration::from_secs(60);
const ANSWER_PROMPT: &str = "Reply with exactly: OK";

/// The installed agent, or the check failing with its install fix.
fn installed(check: Check, backend: AssistantBackend) -> Result<PathBuf, CheckResult> {
    super::executable(backend).ok_or_else(|| {
        CheckResult::failed(
            check,
            format!("{} isn't installed", name(backend)),
            Some(Fix::Install(backend)),
        )
    })
}

fn answers(check: Check, backend: AssistantBackend, settings: &AssistantSettings) -> CheckResult {
    if backend == AssistantBackend::ClaudeApi {
        return api_answers(check, settings);
    }
    let path = match installed(check, backend) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let mut command = agent_command(&path);
    command.current_dir(scratch());
    match backend {
        AssistantBackend::ClaudeCode => {
            command.args([
                "-p",
                ANSWER_PROMPT,
                "--output-format",
                "stream-json",
                "--verbose",
            ]);
            command.args([
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
            ]);
            if let Some(model) = &settings.claude_code_model {
                command.args(["--model", model]);
            }
        }
        AssistantBackend::Codex => {
            command.args(codex_base(&scratch()));
            if let Some(model) = &settings.codex_model {
                command.args(["--model", model]);
            }
            command.arg(ANSWER_PROMPT);
        }
        AssistantBackend::OpenCode => {
            // Standalone, as tasks run it, with the person's own key when
            // one is chosen.
            let model =
                super::providers::opencode_check(&mut command, settings, serde_json::json!({}));
            command.args(["run", "--standalone", "--format", "json", "--archived"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(ANSWER_PROMPT);
        }
        AssistantBackend::ClaudeApi => unreachable!("answered above"),
    }
    let ran = match run::bounded(command, ANSWER_LIMIT) {
        Ok(ran) => ran,
        Err(error) => {
            return CheckResult::failed(
                check,
                format!("{} didn't start: {error}", name(backend)),
                Some(Fix::Install(backend)),
            );
        }
    };
    let events = parse_events(&ran.stdout);
    let text = answer_text(&events);
    if text.to_ascii_uppercase().contains("OK") {
        let mut detail = format!(
            "Replied “{}” in {:.1} s",
            run::shorten(text.trim(), 40),
            ran.took.as_secs_f32()
        );
        // Without its sandbox a job gets no shell; say so here.
        if backend == AssistantBackend::ClaudeCode
            && let Err(why) = crate::assistant::provider::claude_code_sandbox()
        {
            detail.push_str(&format!(". Jobs run without a shell: {why}"));
        }
        return CheckResult::passed(check, detail);
    }
    let problem = event_error(&events).unwrap_or_else(|| {
        if ran.timed_out() {
            format!("No answer within {} s", ANSWER_LIMIT.as_secs())
        } else if text.trim().is_empty() {
            ran.last_words()
        } else {
            format!("Replied “{}”", run::shorten(text.trim(), 80))
        }
    });
    let fix = if signed_out(&problem) {
        Some(sign_in_fix(backend))
    } else if backend == AssistantBackend::OpenCode && !crate::desktop::opencode_service::running()
    {
        Some(Fix::StartOpenCode)
    } else {
        Some(Fix::Retry)
    };
    CheckResult::failed(check, problem, fix)
}

fn sign_in_fix(backend: AssistantBackend) -> Fix {
    match backend {
        AssistantBackend::ClaudeApi => Fix::AddApiKey,
        backend => Fix::SignIn(backend),
    }
}

/// Whether an agent's error says it isn't signed in.
pub fn signed_out(problem: &str) -> bool {
    let problem = problem.to_ascii_lowercase();
    [
        "not logged in",
        "please run /login",
        "/login",
        "log in",
        "login",
        "sign in",
        "invalid api key",
        "unauthorized",
        "401",
        "authentication",
        "oauth token",
    ]
    .iter()
    .any(|marker| problem.contains(marker))
}

/// `codex exec`, read-only, without a repository or saved session.
fn codex_base(folder: &Path) -> Vec<String> {
    [
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ephemeral",
        "--sandbox",
        "read-only",
        "--cd",
    ]
    .map(str::to_owned)
    .into_iter()
    .chain([folder.to_string_lossy().into_owned()])
    .collect()
}

/// One JSON value a line, as each agent's event stream prints them.
fn parse_events(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str(line.trim()).ok())
        .collect()
}

/// The agent's reply text, from any of the three event streams.
pub fn answer_text(events: &[Value]) -> String {
    let mut text = String::new();
    for event in events {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            // claude -p: the final result.
            "result" => {
                if let Some(result) = event.get("result").and_then(Value::as_str) {
                    return result.to_owned();
                }
            }
            // codex exec --json
            "item.completed" => {
                let item = &event["item"];
                if item.get("type").and_then(Value::as_str) == Some("agent_message")
                    && let Some(message) = item.get("text").and_then(Value::as_str)
                {
                    text = message.to_owned();
                }
            }
            // opencode run --format json
            "text" => {
                if let Some(part) = event["part"].get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
            _ => {}
        }
    }
    text
}

/// The first error an event stream reports, for a person.
pub fn event_error(events: &[Value]) -> Option<String> {
    events.iter().find_map(|event| {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = match kind {
            // claude -p
            "result" if event.get("is_error").and_then(Value::as_bool) == Some(true) => event
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    event
                        .get("subtype")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
            // codex: `error` events and a failed turn; opencode: `error`.
            "error" => event
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| event["error"].get("message").and_then(Value::as_str))
                .map(str::to_owned),
            "turn.failed" => event["error"]
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        }?;
        Some(run::shorten(message.trim(), 240))
    })
}

fn api_answers(check: Check, settings: &AssistantSettings) -> CheckResult {
    let Some(key) = crate::assistant::credentials::api_key() else {
        return CheckResult::failed(check, "No Anthropic API key is saved", Some(Fix::AddApiKey));
    };
    const URL: &str = "https://api.anthropic.com/v1/messages";
    let agent = crate::net::agent(URL, |config| {
        config
            .timeout_global(Some(ANSWER_LIMIT))
            .http_status_as_error(false)
    });
    let body = serde_json::json!({
        "model": settings.claude_api_model,
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": ANSWER_PROMPT }],
    });
    let started = Instant::now();
    let response = agent
        .post(URL)
        .header("x-api-key", key.as_str())
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .send(body.to_string());
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            return CheckResult::failed(
                check,
                crate::net::unreachable(URL, &error),
                Some(Fix::Retry),
            );
        }
    };
    let status = response.status().as_u16();
    let reply: Value = response
        .body_mut()
        .read_to_string()
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    if status != 200 {
        let message = reply["error"]["message"]
            .as_str()
            .unwrap_or("the API refused")
            .to_owned();
        let fix = if status == 401 || status == 403 {
            Fix::AddApiKey
        } else {
            Fix::Retry
        };
        return CheckResult::failed(check, format!("{status}: {message}"), Some(fix));
    }
    let text = reply["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>();
    if text.to_ascii_uppercase().contains("OK") {
        CheckResult::passed(
            check,
            format!(
                "Replied “{}” in {:.1} s",
                run::shorten(text.trim(), 40),
                started.elapsed().as_secs_f32()
            ),
        )
    } else {
        CheckResult::failed(
            check,
            format!("Replied “{}”", run::shorten(text.trim(), 80)),
            Some(Fix::Retry),
        )
    }
}

/// agent-desktop, or the check failing with its install fix.
fn agent_desktop(check: Check) -> Result<PathBuf, CheckResult> {
    search::find("agent-desktop").ok_or_else(|| {
        CheckResult::failed(
            check,
            "agent-desktop isn't installed",
            Some(Fix::InstallAgentDesktop),
        )
    })
}

/// What a permissions reply means for a check.
fn judge_grants(check: Check, grants: desktop::Grants, who: &str) -> CheckResult {
    match (grants.accessibility, grants.screen_recording) {
        (true, true) => CheckResult::passed(
            check,
            format!("Accessibility and Screen Recording are granted ({who})"),
        ),
        (false, _) if cfg!(target_os = "linux") => CheckResult::failed(
            check,
            format!(
                "agent-desktop can't read apps ({who}): turn on assistive technologies \
                 (AT-SPI), e.g. gsettings set org.gnome.desktop.interface \
                 toolkit-accessibility true, then sign out and in"
            ),
            None,
        ),
        (false, _) => CheckResult::failed(
            check,
            format!("agent-desktop has no Accessibility ({who}). Switch Hex on in System Settings"),
            Some(Fix::GuidePermissions),
        ),
        (true, false) => CheckResult::failed(
            check,
            format!(
                "agent-desktop has no Screen Recording ({who}), so agents can't take screenshots"
            ),
            Some(Fix::GuidePermissions),
        ),
    }
}

fn desktop_permissions(check: Check) -> CheckResult {
    let path = match agent_desktop(check) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let grants = super::agent_desktop_grants(&path);
    // On Linux agent-desktop reads apps through the AT-SPI accessibility
    // bus; when it can't, a missing bus is the likely reason.
    #[cfg(target_os = "linux")]
    if !grants.as_ref().is_ok_and(|grants| grants.accessibility)
        && let Some(Err(why)) = atspi_bus()
    {
        return CheckResult::failed(check, why, Some(Fix::Retry));
    }
    match grants {
        Ok(grants) => judge_grants(check, grants, "started by Hex"),
        Err(error) => CheckResult::failed(
            check,
            format!("agent-desktop didn't answer: {error}"),
            Some(Fix::InstallAgentDesktop),
        ),
    }
}

/// Whether the session's AT-SPI accessibility bus answers (`org.a11y.Bus`
/// on the session D-Bus); `None` when there's nothing to ask it with.
#[cfg(target_os = "linux")]
fn atspi_bus() -> Option<Result<(), String>> {
    const MISSING: &str = "The accessibility bus (AT-SPI) isn't running, so agent-desktop \
                           can't read apps. Install at-spi2-core and sign in to a desktop \
                           session again";
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none()
        && std::env::var_os("XDG_RUNTIME_DIR")
            .is_none_or(|dir| !Path::new(&dir).join("bus").exists())
    {
        return Some(Err(
            "There's no desktop session bus (D-Bus), so agent-desktop can't read apps".into(),
        ));
    }
    let (program, args): (_, &[&str]) = if let Some(gdbus) = search::find("gdbus") {
        (
            gdbus,
            &[
                "call",
                "--session",
                "--dest",
                "org.a11y.Bus",
                "--object-path",
                "/org/a11y/bus",
                "--method",
                "org.a11y.Bus.GetAddress",
            ],
        )
    } else if let Some(send) = search::find("dbus-send") {
        (
            send,
            &[
                "--session",
                "--print-reply",
                "--dest=org.a11y.Bus",
                "/org/a11y/bus",
                "org.a11y.Bus.GetAddress",
            ],
        )
    } else {
        return None;
    };
    let mut command = crate::desktop::background_process::command(program);
    command.args(args);
    let ran = run::bounded(command, Duration::from_secs(5)).ok()?;
    Some(if ran.success() {
        Ok(())
    } else {
        Err(MISSING.into())
    })
}

fn round_trip(check: Check) -> CheckResult {
    let path = match agent_desktop(check) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let call = |args: &[&str]| -> Result<Value, String> {
        match super::desktop_call(&path, args)? {
            desktop::Reply::Ok(data) => Ok(data),
            desktop::Reply::Failed { code, message } => Err(format!("{code}: {message}")),
        }
    };
    let apps = match call(&["list-apps"]) {
        Ok(data) => desktop::apps(&data),
        Err(error) => return failed_round_trip(check, "list the running apps", &error),
    };
    if apps.is_empty() {
        return failed_round_trip(check, "list the running apps", "it listed none");
    }
    // Only macOS has a Dock.
    let dock = if cfg!(target_os = "macos") {
        let dock = match call(&["snapshot", "--surface", "dock", "--skeleton", "--compact"]) {
            Ok(data) => data.get("ref_count").and_then(Value::as_u64).unwrap_or(0),
            Err(error) => return failed_round_trip(check, "read the Dock", &error),
        };
        if dock == 0 {
            return failed_round_trip(check, "read the Dock", "it found nothing in it");
        }
        Some(dock)
    } else {
        None
    };
    let windows = match call(&["list-windows"]) {
        Ok(data) => desktop::windows(&data),
        Err(error) => return failed_round_trip(check, "list the windows", &error),
    };
    let visible: Vec<_> = windows.iter().filter(|window| window.visible).collect();
    let outlined = visible.iter().find(|window| {
        computer::window_frame(&crate::assistant::TaskWindow {
            app: window.app.clone(),
            window_id: Some(window.id.clone()),
        })
        .is_some()
    });
    if !visible.is_empty() && outlined.is_none() {
        return CheckResult::failed(
            check,
            "Hex can't find agent-desktop's windows on screen, so task outlines won't show",
            Some(Fix::Retry),
        );
    }
    let dock = dock
        .map(|dock| format!(", {dock} items in the Dock"))
        .unwrap_or_default();
    CheckResult::passed(
        check,
        format!(
            "{} apps running, {} windows{dock}",
            apps.len(),
            windows.len()
        ),
    )
}

fn failed_round_trip(check: Check, what: &str, why: &str) -> CheckResult {
    let fix = if why.contains("PERM") || why.to_ascii_lowercase().contains("permission") {
        Fix::GuidePermissions
    } else {
        Fix::Retry
    };
    CheckResult::failed(
        check,
        format!("agent-desktop couldn't {what}: {why}"),
        Some(fix),
    )
}

/// The prompt that has an agent call `desktop_status` under its own name
/// for the tool.
fn status_prompt(tool: &str) -> String {
    format!(
        "Call the {tool} tool once, with no arguments. Then reply with only the value of \
         permissions.accessibility.state from its result. Do nothing else."
    )
}

fn through_agent(
    check: Check,
    backend: AssistantBackend,
    settings: &AssistantSettings,
) -> CheckResult {
    let path = match installed(check, backend) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let Some(server) = super::mcp_server(&[]) else {
        return CheckResult::failed(
            check,
            "agent-desktop isn't installed",
            Some(Fix::InstallAgentDesktop),
        );
    };
    let mut command = agent_command(&path);
    command.current_dir(scratch());
    match backend {
        AssistantBackend::ClaudeCode => {
            let tool = format!("mcp__{}__desktop_status", super::MCP_NAME);
            command.args([
                "-p",
                &status_prompt(&tool),
                "--output-format",
                "stream-json",
            ]);
            command.args(["--verbose", "--tools", "", "--no-session-persistence"]);
            command.args([
                "--mcp-config",
                &server.claude_config(),
                "--strict-mcp-config",
            ]);
            command.args(["--allowedTools", &tool]);
            if let Some(model) = &settings.claude_code_model {
                command.args(["--model", model]);
            }
        }
        AssistantBackend::Codex => {
            command.args(codex_base(&scratch()));
            command.args(server.codex_args(&["desktop_status"]));
            if let Some(model) = &settings.codex_model {
                command.args(["--model", model]);
            }
            command.arg(status_prompt("desktop_status"));
        }
        AssistantBackend::OpenCode => {
            let tool = format!("{}_desktop_status", super::MCP_NAME);
            let config = serde_json::json!({
                "mcp": server.opencode_config(),
                "tools": { "*": false, tool.as_str(): true },
            });
            let model = super::providers::opencode_check(&mut command, settings, config);
            // A private server, so the config above is its own.
            command.args(["run", "--standalone", "--format", "json", "--archived"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(status_prompt(&tool));
        }
        AssistantBackend::ClaudeApi => {
            return CheckResult::passed(check, "Hex runs agent-desktop for the Claude API itself");
        }
    }
    let ran = match run::bounded(command, MCP_LIMIT) {
        Ok(ran) => ran,
        Err(error) => {
            return CheckResult::failed(
                check,
                format!("{} didn't start: {error}", name(backend)),
                Some(Fix::Retry),
            );
        }
    };
    if let Some(status) = desktop::status_in_events(&ran.stdout) {
        return match desktop::grants(&status) {
            Some(grants) => {
                let who = format!("started by {} from Hex", name(backend));
                let mut result = judge_grants(check, grants, &who);
                if result.ok() {
                    result.detail = format!(
                        "Called agent-desktop in {:.1} s; it has Accessibility and Screen Recording",
                        ran.took.as_secs_f32()
                    );
                }
                result
            }
            None => CheckResult::failed(
                check,
                "agent-desktop answered without its permissions",
                Some(Fix::Retry),
            ),
        };
    }
    let events = parse_events(&ran.stdout);
    let problem = mcp_error(&events)
        .or_else(|| event_error(&events))
        .unwrap_or_else(|| {
            if ran.timed_out() {
                format!("No reply within {} s", MCP_LIMIT.as_secs())
            } else {
                let said = answer_text(&events);
                if said.trim().is_empty() {
                    format!("It never called agent-desktop. {}", ran.last_words())
                } else {
                    format!(
                        "It never called agent-desktop; it said “{}”",
                        run::shorten(said.trim(), 80)
                    )
                }
            }
        });
    let fix = if signed_out(&problem) {
        sign_in_fix(backend)
    } else {
        Fix::Retry
    };
    CheckResult::failed(check, problem, Some(fix))
}

/// A failed MCP tool call in Codex's events ("requires approval", a server
/// that didn't start).
fn mcp_error(events: &[Value]) -> Option<String> {
    events.iter().find_map(|event| {
        let item = event.get("item")?;
        (item.get("type").and_then(Value::as_str) == Some("mcp_tool_call"))
            .then(|| item["error"].get("message").and_then(Value::as_str))
            .flatten()
            .map(|message| format!("The desktop tool call failed: {message}"))
    })
}

/// The model OpenCode used last (`~/.local/state/opencode/model.json`),
/// which a private server doesn't otherwise know.
pub fn opencode_recent_model() -> Option<String> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::app_paths::home_dir().map(|home| home.join(".local/state")))?;
    let text = std::fs::read_to_string(state.join("opencode/model.json")).ok()?;
    recent_model(&text)
}

/// `provider/model` of the first `recent` entry in OpenCode's model state.
pub fn recent_model(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let recent = value.get("recent")?.as_array()?.first()?;
    let provider = recent.get("providerID")?.as_str()?;
    let model = recent.get("modelID")?.as_str()?;
    Some(format!("{provider}/{model}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(text: &str) -> Vec<Value> {
        parse_events(text)
    }

    #[test]
    fn each_agents_answer_is_read_from_its_events() {
        let claude = r#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"OK"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"OK"}"#;
        let codex = r#"{"type":"thread.started","thread_id":"t"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
{"type":"turn.completed","usage":{}}"#;
        let opencode = r#"{"type":"step_start","part":{"type":"step-start"}}
{"type":"text","part":{"type":"text","text":"OK"}}
{"type":"step_finish","part":{"type":"step-finish","reason":"stop"}}"#;
        for stream in [claude, codex, opencode] {
            assert_eq!(answer_text(&events(stream)), "OK");
            assert_eq!(event_error(&events(stream)), None);
        }
    }

    #[test]
    fn each_agents_errors_are_read_from_its_events() {
        let claude = r#"{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key · Please run /login"}"#;
        let codex = r#"{"type":"error","message":"unexpected status 401 Unauthorized"}
{"type":"turn.failed","error":{"message":"unexpected status 401 Unauthorized"}}"#;
        let opencode = r#"{"type":"error","error":{"type":"provider.free-limit","message":"Free usage limit reached."}}"#;
        let claude = event_error(&events(claude)).unwrap();
        assert!(signed_out(&claude), "{claude}");
        let codex = event_error(&events(codex)).unwrap();
        assert!(signed_out(&codex), "{codex}");
        let opencode = event_error(&events(opencode)).unwrap();
        assert_eq!(opencode, "Free usage limit reached.");
        assert!(!signed_out(&opencode));
    }

    #[test]
    fn a_refused_desktop_tool_call_is_named() {
        let codex = r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"agent-desktop","tool":"desktop_status","arguments":{},"result":null,"error":{"message":"MCP tool call requires approval, but approval policy is never"},"status":"failed"}}"#;
        assert_eq!(
            mcp_error(&events(codex)).as_deref(),
            Some(
                "The desktop tool call failed: MCP tool call requires approval, but approval policy is never"
            )
        );
    }

    #[test]
    fn opencodes_last_model_is_its_first_recent_one() {
        let state = r#"{"recent":[{"providerID":"deepseek","modelID":"deepseek-flash"},{"providerID":"opencode","modelID":"x-free"}],"favorite":[]}"#;
        assert_eq!(
            recent_model(state).as_deref(),
            Some("deepseek/deepseek-flash")
        );
        assert_eq!(recent_model(r#"{"recent":[]}"#), None);
        assert_eq!(recent_model("nonsense"), None);
    }

    #[test]
    fn each_backend_gets_its_checks_with_their_prerequisites() {
        assert_eq!(
            Check::for_backend(AssistantBackend::Codex, false),
            [
                Check::HexPermissions,
                Check::Answers(AssistantBackend::Codex),
                Check::DesktopPermissions,
                Check::DesktopRoundTrip,
                Check::DesktopThroughAgent(AssistantBackend::Codex),
            ]
        );
        let browser = Check::for_backend(AssistantBackend::Codex, true)[5..].to_vec();
        if cfg!(windows) {
            // No Lightpanda for Windows.
            assert_eq!(
                browser,
                [
                    Check::BrowserRoundTrip,
                    Check::BrowserThroughAgent(AssistantBackend::Codex),
                ]
            );
        } else {
            assert_eq!(
                browser,
                [
                    Check::BrowserRoundTrip,
                    Check::LightBrowser,
                    Check::BrowserThroughAgent(AssistantBackend::Codex),
                ]
            );
        }
        assert_eq!(
            Check::BrowserThroughAgent(AssistantBackend::Codex).needs(),
            [
                Check::Answers(AssistantBackend::Codex),
                Check::BrowserRoundTrip
            ]
        );
        assert!(
            Check::for_backend(AssistantBackend::ClaudeApi, true)
                .contains(&Check::BrowserThroughAgent(AssistantBackend::ClaudeApi))
        );
        assert!(
            !Check::for_backend(AssistantBackend::ClaudeApi, true)
                .contains(&Check::DesktopThroughAgent(AssistantBackend::ClaudeApi))
        );
        assert_eq!(
            Check::DesktopThroughAgent(AssistantBackend::Codex).needs(),
            [
                Check::Answers(AssistantBackend::Codex),
                Check::DesktopPermissions
            ]
        );
    }

    #[test]
    fn a_check_whose_prerequisite_failed_fails_with_its_fix() {
        // Without a key the Claude API fails at once, offline, and what
        // needs it never runs.
        if crate::assistant::credentials::has_api_key() {
            return;
        }
        let settings = AssistantSettings::default();
        let api = AssistantBackend::ClaudeApi;
        let mut reported = Vec::new();
        let results = self_tests(
            &[Check::Answers(api), Check::DesktopThroughAgent(api)],
            &settings,
            |result| reported.push((result.check, result.state)),
        );
        assert_eq!(results[0].fix, Some(Fix::AddApiKey));
        assert_eq!(results[1].fix, Some(Fix::AddApiKey));
        assert!(
            results[1].detail.starts_with("Needs"),
            "{}",
            results[1].detail
        );
        assert_eq!(
            reported,
            [
                (Check::Answers(api), CheckState::Running),
                (Check::Answers(api), CheckState::Failed),
                (Check::DesktopThroughAgent(api), CheckState::Failed),
            ]
        );
    }
}
