//! The onboarding's Browser page: agent-browser found (or installed with
//! Homebrew), its own hidden browser downloaded, and its self-tests green,
//! so web tasks run in browsers of their own instead of the person's.

use super::super::browser_shim::{self as bshim, CheckResult};
use super::super::{Done, Job, Ui};
use super::SetupFlow;
use crate::assistant::backends::CheckState;
use crate::assistant::browser::AgentBrowser;

/// The Browser page's state.
#[derive(Default)]
pub(in crate::rune_ui) struct BrowserStep {
    /// agent-browser as last read.
    pub(in crate::rune_ui) found: Option<AgentBrowser>,
    pub(in crate::rune_ui) reading: bool,
    pub(in crate::rune_ui) installing: bool,
    /// Why the last install failed.
    pub(in crate::rune_ui) error: Option<String>,
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

    /// Ready, and every self-test passed.
    pub(in crate::rune_ui) fn done(&self) -> bool {
        self.ready() && !self.checks.is_empty() && self.checks.iter().all(CheckResult::ok)
    }
}

/// Reads agent-browser again.
pub(in crate::rune_ui) fn read(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.browser.reading || flow.browser.installing {
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

/// `brew install agent-browser`, then its browser.
pub(in crate::rune_ui) fn install(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.browser.installing {
        return;
    }
    flow.browser.installing = true;
    flow.browser.error = None;
    ui.chats.jobs.run(Job::InstallBrowser);
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
    flow.browser.checks = bshim::for_backend(settings.backend)
        .into_iter()
        .map(CheckResult::pending)
        .collect();
    ui.chats.jobs.run(Job::BrowserChecks(Box::new(settings)));
}

/// What a failed browser check offers.
pub(in crate::rune_ui) fn apply(ui: &mut Ui, fix: bshim::Fix) {
    match fix {
        bshim::Fix::InstallAgentBrowser => install(ui),
        bshim::Fix::Retry => run_checks(ui),
    }
}

/// A browser job's answer. True when it was one.
pub(super) fn receive(ui: &mut Ui, done: &Done) -> bool {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return false;
    };
    let here = flow.page == super::Page::Browser;
    let step = &mut flow.browser;
    match done {
        Done::Browser(found) => {
            step.reading = false;
            step.found = Some(found.clone());
            if here && step.ready() && step.checks.is_empty() {
                run_checks(ui);
            }
        }
        Done::BrowserInstalled(result) => {
            step.installing = false;
            step.error = result.clone().err();
            step.checks.clear();
            read(ui);
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
pub(in crate::rune_ui) fn perform_checks(
    settings: &crate::app_settings::AssistantSettings,
    send: &dyn Fn(Done),
) {
    for check in bshim::for_backend(settings.backend) {
        let mut running = CheckResult::pending(check);
        running.state = CheckState::Running;
        send(Done::BrowserCheck(running));
        send(Done::BrowserCheck(bshim::self_test(check, settings)));
    }
    send(Done::BrowserChecked);
}

/// The page for scenes: agent-browser found and ready, one check passed,
/// the next failed with its fix (or every check passed).
pub(in crate::rune_ui) fn fixture(flow: &mut SetupFlow, backend: crate::app_settings::AssistantBackend, all_passed: bool) {
    use std::time::Duration;
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
            bshim::Check::BrowserRoundTrip,
            CheckState::Ok,
            "Opened example.com and read its title in 1.8 s",
            None,
        ),
        if all_passed {
            result(
                bshim::Check::BrowserThroughAgent(backend),
                CheckState::Ok,
                "Opened a page through agent-browser in 7.2 s",
                None,
            )
        } else {
            result(
                bshim::Check::BrowserThroughAgent(backend),
                CheckState::Failed,
                "The agent didn't call agent-browser within 60 s",
                Some(bshim::Fix::Retry),
            )
        },
    ];
}
