//! The onboarding's Browser page: agent-browser found (or installed with
//! one click, its own Chrome with it), and its self-tests green, so web
//! tasks run in browsers of their own instead of the person's. Signing in
//! to sites in Hex's own browser is offered, never required.

use std::time::Duration;

use super::super::{Done, Job, Ui};
use super::{Page, SetupFlow};
use crate::app_settings::AssistantSettings;
use crate::assistant::backends::{self, Check, CheckResult, CheckState, Fix};
use crate::assistant::browser::AgentBrowser;

/// The Browser page's state.
#[derive(Default)]
pub(in crate::rune_ui) struct BrowserStep {
    /// agent-browser as last read.
    pub(in crate::rune_ui) found: Option<AgentBrowser>,
    pub(in crate::rune_ui) reading: bool,
    pub(in crate::rune_ui) checks: Vec<CheckResult>,
    pub(in crate::rune_ui) checks_running: bool,
}

impl BrowserStep {
    /// Installed, with its own browser downloaded.
    pub(in crate::rune_ui) fn ready(&self) -> bool {
        self.found
            .as_ref()
            .is_some_and(|found| found.path.is_some() && found.browser_ready)
    }

    /// Ready, and every self-test passed but the optional Lightpanda one.
    pub(in crate::rune_ui) fn done(&self) -> bool {
        self.ready() && !self.checks.is_empty() && self.checks.iter().all(passed_or_optional)
    }
}

/// Whether `result` passed, or is the optional Lightpanda check (which
/// never holds a page back).
pub(in crate::rune_ui) fn passed_or_optional(result: &CheckResult) -> bool {
    result.ok() || result.check == Check::LightBrowser
}

/// The browser's own checks, in order.
pub(in crate::rune_ui) fn checks_for(settings: &AssistantSettings) -> Vec<Check> {
    backends::Check::for_backend(settings.backend, true)
        .into_iter()
        .filter(|check| {
            matches!(
                check,
                Check::BrowserRoundTrip | Check::LightBrowser | Check::BrowserThroughAgent(_)
            )
        })
        .collect()
}

/// Reads agent-browser again.
pub(in crate::rune_ui) fn read(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.browser.reading || flow.fixing == Some(Fix::InstallAgentBrowser) {
        return;
    }
    flow.browser.reading = true;
    ui.chats.jobs.run(Job::Browser);
}

/// Arriving at the page: its self-tests run once agent-browser is ready.
pub(in crate::rune_ui) fn arrive(ui: &mut Ui) {
    let ready = ui
        .chats
        .setup
        .as_ref()
        .is_some_and(|flow| flow.browser.ready() && flow.browser.checks.is_empty());
    if ready {
        run_checks(ui);
    } else {
        read(ui);
    }
}

/// Runs the browser's self-tests against the chosen agent.
pub(in crate::rune_ui) fn run_checks(ui: &mut Ui) {
    let settings = ui.settings.assistant.clone();
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.browser.checks_running {
        return;
    }
    flow.browser.checks_running = true;
    flow.browser.checks = checks_for(&settings)
        .into_iter()
        .map(|check| CheckResult {
            check,
            state: CheckState::Pending,
            detail: String::new(),
            fix: None,
            took: Duration::ZERO,
        })
        .collect();
    ui.chats.jobs.run(Job::BrowserChecks(Box::new(settings)));
}

/// An install of agent-browser finished: look again, and test it.
pub(super) fn installed(ui: &mut Ui) {
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.browser.checks.clear();
    }
    read(ui);
}

/// A browser job's answer. True when it was one.
pub(super) fn receive(ui: &mut Ui, done: &Done) -> bool {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return false;
    };
    let here = flow.page == Page::Browser;

    let step = &mut flow.browser;
    match done {
        Done::Browser(found, _, _) => {
            step.reading = false;
            step.found = Some(found.clone());
            if here && step.ready() && step.checks.is_empty() {
                run_checks(ui);
            }
            // Settings keeps it too.
            return false;
        }
        Done::BrowserCheck(result) => {
            if let Some(slot) = step
                .checks
                .iter_mut()
                .find(|slot| slot.check == result.check)
            {
                *slot = result.clone();
            }
        }
        Done::BrowserChecked => step.checks_running = false,
        _ => return false,
    }
    true
}

/// Runs the browser checks off the window's thread, reporting each.
pub(in crate::rune_ui) fn perform_checks(settings: &AssistantSettings, send: &dyn Fn(Done)) {
    backends::self_tests(&checks_for(settings), settings, |result| {
        send(Done::BrowserCheck(result.clone()))
    });
    send(Done::BrowserChecked);
}

/// The page for scenes: agent-browser found and ready, one check passed,
/// the next failed with its fix (or every check passed).
pub(in crate::rune_ui) fn fixture(
    flow: &mut SetupFlow,
    backend: crate::app_settings::AssistantBackend,
    all_passed: bool,
) {
    flow.browser.found = Some(AgentBrowser {
        path: Some("/opt/homebrew/bin/agent-browser".into()),
        version: Some("0.9.3".into()),
        browser_ready: true,
    });
    let result = |check, state, detail: &str, fix| CheckResult {
        check,
        state,
        detail: detail.into(),
        fix,
        took: Duration::ZERO,
    };
    flow.browser.checks = vec![
        result(
            Check::BrowserRoundTrip,
            CheckState::Ok,
            "Opened example.com and read its title in 1.8 s",
            None,
        ),
        if all_passed {
            result(
                Check::LightBrowser,
                CheckState::Ok,
                "Read example.com in Lightpanda in 0.3 s",
                None,
            )
        } else {
            result(
                Check::LightBrowser,
                CheckState::Failed,
                "Lightpanda isn't installed, so clean browsers run Chrome (about ten times the memory)",
                Some(Fix::InstallLightpanda),
            )
        },
        if all_passed {
            result(
                Check::BrowserThroughAgent(backend),
                CheckState::Ok,
                "Opened a page through agent-browser in 7.2 s",
                None,
            )
        } else {
            result(
                Check::BrowserThroughAgent(backend),
                CheckState::Failed,
                "The agent didn't call agent-browser within 60 s",
                Some(Fix::Retry),
            )
        },
    ];
}
