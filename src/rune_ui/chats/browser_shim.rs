//! TEMPORARY SHIM: remove at integration.
//!
//! Stand-ins, named as the browser engineer's contract names them, for the
//! agent-browser self-tests the onboarding's Browser page runs:
//! `backends::Check::BrowserRoundTrip`, `backends::Check::BrowserThroughAgent(B)`,
//! and `backends::Fix::InstallAgentBrowser`, which aren't in this worktree.
//!
//! At integration: delete this file and `mod browser_shim;`, and in
//! `setup/browser.rs`, `sheet/browser.rs`, `jobs.rs`, and `fixture.rs` read
//! `browser_shim::Check` as `backends::Check`, `browser_shim::Fix` as
//! `backends::Fix`, `browser_shim::CheckResult` as `backends::CheckResult`,
//! and `browser_shim::self_test` as `backends::self_test` (then give
//! `setup::check_title` and `setup::fix_label` the new variants' rows).

use std::time::Duration;

use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::backends::CheckState;

/// One browser self-test.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Check {
    /// agent-browser opens a page in its own hidden browser and reads it.
    BrowserRoundTrip,
    /// The agent drives agent-browser through MCP.
    BrowserThroughAgent(AssistantBackend),
}

/// What the Browser page offers for a failed browser check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Fix {
    /// `brew install agent-browser`, then `agent-browser install`.
    InstallAgentBrowser,
    Retry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) struct CheckResult {
    pub(in crate::rune_ui) check: Check,
    pub(in crate::rune_ui) state: CheckState,
    pub(in crate::rune_ui) detail: String,
    pub(in crate::rune_ui) fix: Option<Fix>,
    pub(in crate::rune_ui) took: Duration,
}

impl CheckResult {
    pub(in crate::rune_ui) fn ok(&self) -> bool {
        self.state == CheckState::Ok
    }

    pub(in crate::rune_ui) fn pending(check: Check) -> Self {
        Self {
            check,
            state: CheckState::Pending,
            detail: String::new(),
            fix: None,
            took: Duration::ZERO,
        }
    }
}

/// The browser checks for `backend`, in order (the Claude API's tasks get
/// the browser from Hex itself).
pub(in crate::rune_ui) fn for_backend(backend: AssistantBackend) -> Vec<Check> {
    let mut checks = vec![Check::BrowserRoundTrip];
    if backend != AssistantBackend::ClaudeApi {
        checks.push(Check::BrowserThroughAgent(backend));
    }
    checks
}

/// Until the real checks land: what can be told without running one.
pub(in crate::rune_ui) fn self_test(check: Check, settings: &AssistantSettings) -> CheckResult {
    let _ = settings;
    let found = crate::assistant::browser::agent_browser();
    let (detail, fix) = if found.path.is_none() {
        ("agent-browser isn't installed", Fix::InstallAgentBrowser)
    } else if !found.browser_ready {
        ("agent-browser's own browser isn't downloaded", Fix::InstallAgentBrowser)
    } else {
        ("This build of Hex can't test agent-browser yet", Fix::Retry)
    };
    CheckResult {
        check,
        state: CheckState::Failed,
        detail: detail.into(),
        fix: Some(fix),
        took: Duration::ZERO,
    }
}
