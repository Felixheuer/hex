//! Setup's self-tests: each proves one piece really works from Agent-Desk, and
//! says what to do when it doesn't. They only read: the agents are asked to
//! reply "OK" or to call the guard's `desktop_status`, and Cua Driver lists
//! apps and windows; nothing is clicked, typed, or quit.
//!
//! Run from Agent-Desk, so macOS judges Agent-Desk.app's permissions: Cua
//! Driver started by Agent-Desk (directly, or behind the guard of an agent
//! Agent-Desk started) acts with Agent-Desk's Accessibility and Screen Recording.

mod answer;
mod browser;
mod driver;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

use super::{agent_command, desktop, name, run, search};
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::computer::{self, Permission};

use answer::{
    ANSWER_LIMIT, ANSWER_PROMPT, MCP_LIMIT, answers, api_answers, codex_base, installed,
    parse_events, sign_in_fix,
};
pub use answer::{answer_text, event_error, signed_out};
use driver::{desktop_permissions, round_trip, through_agent};

/// One self-test.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Check {
    /// Agent-Desk can read other apps and the screen (its own Accessibility and
    /// Screen Recording), for a question's context.
    HexPermissions,
    /// The agent answers a tiny prompt ("Reply with OK"), 30 s at most.
    Answers(AssistantBackend),
    /// Cua Driver is installed and acts with Agent-Desk's Accessibility and
    /// Screen Recording.
    DesktopPermissions,
    /// Cua Driver lists the running apps and windows.
    DesktopRoundTrip,
    /// The agent calls the guard's desktop tools through MCP
    /// (`desktop_status`), with the grants there too. Not for `ClaudeApi`,
    /// whose tasks Agent-Desk runs the tools for itself.
    DesktopThroughAgent(AssistantBackend),
    /// agent-browser opens a page in a hidden browser of its own, reads its
    /// title, and closes it.
    BrowserRoundTrip,
    /// The agent opens a page through Agent-Desk's browser tool over MCP and reads
    /// its title (for the Claude API, Agent-Desk does, as it does for its tasks).
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
            Self::HexPermissions => "Agent-Desk can read the screen and selected text".into(),
            Self::Answers(backend) => format!("{} answers", name(backend)),
            Self::DesktopPermissions => "Cua Driver has Accessibility and Screen Recording".into(),
            Self::DesktopRoundTrip => "Cua Driver reads apps and windows".into(),
            Self::DesktopThroughAgent(backend) => {
                format!("{} can use the desktop tools", name(backend))
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
    /// What it found, for a person: "Replied “OK” in 2.1 s", "Cua Driver
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
    /// Installs Cua Driver (a checked download; a minute or so).
    InstallDriver,
    /// Looks for a newer Cua Driver and updates it now (`driver::update`).
    UpdateDriver,
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
    /// System Settings at Agent-Desk's own missing permission.
    OpenSettings(Permission),
    /// Run it again (a timeout, a hiccup).
    Retry,
    /// Agent-Desk runs translocated: drop the download mark from the original and
    /// open that (`desktop::permission_doctor`).
    RepairHex,
    /// Remove the other copies of Agent-Desk that confuse its permissions.
    RemoveCopies,
    /// A permission is granted but needs a fresh process.
    RestartHex,
}

impl Fix {
    /// The fix button's label.
    pub fn label(self) -> String {
        match self {
            Self::Install(backend) => format!("Install {}", name(backend)),
            Self::InstallDriver => "Install Cua Driver".into(),
            Self::UpdateDriver => "Update Cua Driver".into(),
            Self::InstallAgentBrowser => "Install agent-browser".into(),
            Self::InstallLightpanda => "Install Lightpanda".into(),
            Self::SignIn(_) => "Sign in".into(),
            Self::UseApiKey(_) => "Use an API key".into(),
            Self::StartOpenCode => "Start OpenCode".into(),
            Self::AddApiKey => "Add API key".into(),
            Self::OpenSettings(permission) => format!("Open {} settings", permission.name()),
            Self::Retry => "Try again".into(),
            Self::RepairHex => "Repair Agent-Desk".into(),
            Self::RemoveCopies => "Remove other copies".into(),
            Self::RestartHex => "Restart Agent-Desk".into(),
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
    // First what keeps any grant from reaching this copy of Agent-Desk.
    #[cfg(target_os = "macos")]
    if let Some(finding) = crate::desktop::permission_doctor::diagnose() {
        use crate::desktop::permission_doctor::{Finding, message};
        tracing::info!(?finding, "the permission doctor found a problem");
        let fix = match finding {
            Finding::Translocated { .. } => Some(Fix::RepairHex),
            Finding::OtherCopies(_) => Some(Fix::RemoveCopies),
            Finding::NotInstalled { .. } => None,
        };
        return CheckResult::failed(check, message(&finding), fix);
    }
    let permissions = computer::permissions_now();
    #[cfg(target_os = "macos")]
    if permissions.accessibility
        && !permissions.screen_recording
        && crate::capture_host::screen_access_now()
            == crate::capture_host::ScreenAccess::NeedsRestart
    {
        return CheckResult::failed(
            check,
            crate::desktop::permission_doctor::NEEDS_RESTART,
            Some(Fix::RestartHex),
        );
    }
    match (permissions.accessibility, permissions.screen_recording) {
        (true, true) => CheckResult::passed(check, "Accessibility and Screen Recording are on"),
        (false, _) => CheckResult::failed(
            check,
            "Agent-Desk needs Accessibility to read the selected text",
            Some(Fix::OpenSettings(Permission::Accessibility)),
        ),
        (true, false) => CheckResult::failed(
            check,
            "Agent-Desk needs Screen Recording to send a screenshot with a question",
            Some(Fix::OpenSettings(Permission::ScreenRecording)),
        ),
    }
}

/// Where the agents run during a check: an empty folder of Agent-Desk's own, so no
/// project's settings or files come into it.
fn scratch() -> PathBuf {
    let folder = std::env::temp_dir().join("hex-assistant-check");
    if let Err(error) = std::fs::create_dir_all(&folder) {
        tracing::warn!(%error, "could not make the self-test folder");
    }
    folder
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
    use super::driver::mcp_error;
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
        let codex = r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"desktop","tool":"desktop_status","arguments":{},"result":null,"error":{"message":"MCP tool call requires approval, but approval policy is never"},"status":"failed"}}"#;
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
