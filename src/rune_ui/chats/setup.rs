//! The assistant's onboarding: a full-window flow in the first run's kit,
//! started when Labs › Assistant turns on before it was ever finished, and
//! again from Chats. Page by page it makes sure an agent answers (chosen,
//! installed, signed in), that Hex and agent-desktop have every permission
//! (each missing one opens its exact System Settings page with Hex's
//! floating guide, and the page waits until macOS reports the grant), runs
//! the hidden browser for web tasks (`browser`), the live self-tests until
//! they're green, makes sure the Action shortcut and dictation shortcuts
//! are free and that a press of each really reaches Hex
//! (`recognition::shortcut_probe`), and ends with the Action shortcut's
//! three gestures, each tried for real (`tutorial`).
//!
//! Nothing shows as working until its check ran and saw it work. Continue
//! waits for a page's checks; Later moves on and leaves that page marked
//! unfinished. Finish waits for an agent that answers and can use
//! agent-desktop.

pub(in crate::rune_ui) mod browser;
pub(in crate::rune_ui) mod resume;
pub(in crate::rune_ui) mod tutorial;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::super::platform::PlatformRequest;

use super::shortcut::{self, Conflict, ConflictFix, Probe};
use super::{Job, Ui};
use crate::app_settings::{AssistantBackend, HotkeySlot};
use crate::assistant::backends::{AgentDesktop, Check, CheckResult, CheckState, Fix, Found};
use crate::assistant::computer::{Permission, Permissions};
use crate::assistant::Event;
use crate::desktop::i18n::{tr, tr_fill};
use crate::desktop::onboarding::{PermissionAction, PermissionKind, PermissionState};

/// How many of a fix's latest lines it keeps.
const FIX_LOG: usize = 6;

/// How often a page that waits on macOS reads the grants again.
const RECHECK: Duration = Duration::from_millis(1500);

/// The flow's pages, in order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Page {
    Welcome,
    Agent,
    /// Screen Recording (and the Microphone and Accessibility, when the
    /// voice setup didn't already grant them), and agent-desktop with its
    /// own grants.
    Desktop,
    /// agent-browser, for web tasks in hidden browsers.
    Browser,
    Checks,
    /// Nothing else takes the Action shortcut, and Hex receives it.
    Shortcut,
    /// The three gestures, tried for real.
    Try,
}

impl Page {
    pub(in crate::rune_ui) const ALL: [Page; 7] = [
        Page::Welcome,
        Page::Agent,
        Page::Desktop,
        Page::Browser,
        Page::Checks,
        Page::Shortcut,
        Page::Try,
    ];

    /// Its stable name, as the saved progress keeps it.
    pub(in crate::rune_ui) fn name(self) -> &'static str {
        match self {
            Page::Welcome => "welcome",
            Page::Agent => "agent",
            Page::Desktop => "desktop",
            Page::Browser => "browser",
            Page::Checks => "checks",
            Page::Shortcut => "shortcut",
            Page::Try => "try",
        }
    }

    pub(in crate::rune_ui) fn named(name: &str) -> Option<Page> {
        Self::ALL.into_iter().find(|page| page.name() == name)
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|page| *page == self).unwrap_or(0)
    }

    fn next(self) -> Option<Page> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Page> {
        self.index().checked_sub(1).map(|index| Self::ALL[index])
    }
}

/// Where a row stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::rune_ui) enum State {
    /// Not checked yet.
    #[default]
    Pending,
    Checking,
    /// Asked macOS (or the person); waiting for it to turn on.
    Waiting,
    /// Checked, and it works.
    Passed,
    /// Checked, and it doesn't.
    Failed,
}

/// A shortcut's press test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum ProbeState {
    /// Waiting for the person to press and release it.
    Waiting,
    Done(Probe),
}

/// A permission the assistant needs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Grant {
    Microphone,
    Accessibility,
    ScreenRecording,
    DesktopAccessibility,
    DesktopScreenRecording,
}

impl Grant {
    pub(in crate::rune_ui) const HEX: [Grant; 3] = [
        Grant::Microphone,
        Grant::Accessibility,
        Grant::ScreenRecording,
    ];
    pub(in crate::rune_ui) const DESKTOP: [Grant; 2] =
        [Grant::DesktopAccessibility, Grant::DesktopScreenRecording];

    pub(in crate::rune_ui) fn key(self) -> &'static str {
        match self {
            Grant::Microphone => "grant-microphone",
            Grant::Accessibility => "grant-accessibility",
            Grant::ScreenRecording => "grant-screen-recording",
            Grant::DesktopAccessibility => "grant-desktop-accessibility",
            Grant::DesktopScreenRecording => "grant-desktop-screen-recording",
        }
    }
}

/// The flow's state.
pub(in crate::rune_ui) struct SetupFlow {
    pub(in crate::rune_ui) page: Page,
    /// Pages left with Later, still unfinished.
    pub(in crate::rune_ui) later: Vec<Page>,
    pub(in crate::rune_ui) detecting: bool,
    pub(in crate::rune_ui) desktop: Option<AgentDesktop>,
    pub(in crate::rune_ui) desktop_checking: bool,
    /// Hex's own Accessibility and Screen Recording, read afresh.
    pub(in crate::rune_ui) permissions: Option<Permissions>,
    pub(in crate::rune_ui) permissions_checking: bool,
    /// Grants whose Allow was pressed and that aren't on yet.
    pub(in crate::rune_ui) asked: Vec<Grant>,
    /// A fix running now (installing, starting OpenCode), and the last
    /// one's failure.
    pub(in crate::rune_ui) fixing: Option<Fix>,
    pub(in crate::rune_ui) fix_error: Option<String>,
    /// The self-tests' results for the chosen agent, in order.
    pub(in crate::rune_ui) checks: Vec<CheckResult>,
    pub(in crate::rune_ui) checks_running: bool,
    /// The agent the checks ran against.
    pub(in crate::rune_ui) checked_backend: Option<AssistantBackend>,
    /// What else takes the tested shortcuts, by shortcut, once read.
    pub(in crate::rune_ui) conflicts: Option<Vec<(HotkeySlot, Conflict)>>,
    conflicts_checking: bool,
    conflicts_checked_at: Option<Instant>,
    /// Each tested shortcut's press test.
    pub(in crate::rune_ui) probes: Vec<(HotkeySlot, ProbeState)>,
    /// The test waiting for a press now.
    probing: Option<(HotkeySlot, Receiver<Probe>)>,
    /// agent-browser and its self-tests.
    pub(in crate::rune_ui) browser: browser::BrowserStep,
    /// The Try page's gestures.
    pub(in crate::rune_ui) tutorial: tutorial::Tutorial,
    checked_at: Option<Instant>,
    /// The pages continue the voice first run, whose "Hex is ready" ends
    /// both.
    pub(in crate::rune_ui) merged: bool,
    /// Shown to someone who set Hex up before the assistant: "New in Hex".
    pub(in crate::rune_ui) new_in_hex: bool,
    /// Reopened after Hex restarted part way: the page says so.
    pub(in crate::rune_ui) resumed: bool,
    /// The running fix's latest lines (an installer's output).
    pub(in crate::rune_ui) fix_log: Vec<String>,
    /// The failed fix's command, to run in Terminal instead.
    pub(in crate::rune_ui) fix_command: Option<String>,
    /// Opened by a hold of the Action shortcut before setup finished.
    pub(in crate::rune_ui) from_hold: bool,
}

impl Default for SetupFlow {
    fn default() -> Self {
        Self {
            page: Page::Welcome,
            later: Vec::new(),
            detecting: false,
            desktop: None,
            desktop_checking: false,
            permissions: None,
            permissions_checking: false,
            asked: Vec::new(),
            fixing: None,
            fix_error: None,
            checks: Vec::new(),
            checks_running: false,
            checked_backend: None,
            conflicts: None,
            conflicts_checking: false,
            conflicts_checked_at: None,
            probes: Vec::new(),
            probing: None,
            browser: browser::BrowserStep::default(),
            tutorial: tutorial::Tutorial::default(),
            checked_at: None,
            merged: false,
            new_in_hex: false,
            resumed: false,
            fix_log: Vec::new(),
            fix_command: None,
            from_hold: false,
        }
    }
}

/// The chosen agent as found, once looked for.
pub(in crate::rune_ui) fn chosen(ui: &Ui) -> Option<&Found> {
    let backend = ui.settings.assistant.backend;
    ui.chats
        .settings
        .found
        .as_ref()?
        .iter()
        .find(|found| found.backend == backend)
}

/// What would make `found` ready.
pub(in crate::rune_ui) fn fix_for(found: &Found) -> Option<Fix> {
    if found.ready {
        return None;
    }
    Some(match (found.backend, found.path.is_some()) {
        (AssistantBackend::ClaudeApi, _) => Fix::AddApiKey,
        (backend, false) => Fix::Install(backend),
        (AssistantBackend::OpenCode, true) => Fix::StartOpenCode,
        (backend, true) => Fix::SignIn(backend),
    })
}

/// A fix's button, translated.
pub(in crate::rune_ui) fn fix_label(fix: Fix) -> String {
    match fix {
        Fix::Install(backend) => tr_fill("Install {}", super::backend_name(backend)),
        Fix::InstallAgentDesktop => tr("Install agent-desktop").into(),
        Fix::InstallAgentBrowser => tr("Install agent-browser").into(),
        Fix::InstallLightpanda => tr("Install Lightpanda").into(),
        Fix::SignIn(_) => tr("Sign in…").into(),
        Fix::StartOpenCode => tr("Start OpenCode").into(),
        Fix::AddApiKey => tr("Add API key").into(),
        Fix::GuidePermissions => tr("Grant permissions…").into(),
        Fix::OpenSettings(Permission::Accessibility) => tr("Open Accessibility…").into(),
        Fix::OpenSettings(Permission::ScreenRecording) => tr("Open Screen Recording…").into(),
        Fix::Retry => tr("Try again").into(),
        // A fix added after this page was written, in its own words.
        #[allow(unreachable_patterns)]
        _ => fix.label(),
    }
}

/// A self-test's row title, translated.
pub(in crate::rune_ui) fn check_title(check: Check) -> String {
    match check {
        Check::HexPermissions => tr("Hex can read the screen and selected text").into(),
        Check::Answers(backend) => tr_fill("{} answers", super::backend_name(backend)),
        Check::DesktopPermissions => {
            tr("agent-desktop has Accessibility and Screen Recording").into()
        }
        Check::DesktopRoundTrip => tr("agent-desktop reads apps and windows").into(),
        Check::DesktopThroughAgent(backend) => {
            tr_fill("{} can use agent-desktop", super::backend_name(backend))
        }
        Check::BrowserRoundTrip => tr("agent-browser opens pages in hidden browsers").into(),
        Check::LightBrowser => tr("Lightpanda reads pages in light browsers").into(),
        Check::BrowserThroughAgent(backend) => {
            tr_fill("{} can use agent-browser", super::backend_name(backend))
        }
    }
}

/// Whether macOS reports `grant` on; `None` until it was read.
fn granted(ui: &Ui, flow: &SetupFlow, grant: Grant) -> Option<bool> {
    let hex = |state: PermissionState| Some(state == PermissionState::Ready);
    match grant {
        Grant::Microphone => hex(ui.platform.permissions.microphone),
        Grant::Accessibility => hex(ui.platform.permissions.accessibility),
        Grant::ScreenRecording => flow.permissions.map(|read| read.screen_recording),
        Grant::DesktopAccessibility => flow
            .desktop
            .as_ref()
            .filter(|found| found.path.is_some())
            .map(|found| found.accessibility),
        Grant::DesktopScreenRecording => flow
            .desktop
            .as_ref()
            .filter(|found| found.path.is_some())
            .map(|found| found.screen_recording),
    }
}

/// How `grant`'s row stands.
pub(in crate::rune_ui) fn grant_state(ui: &Ui, flow: &SetupFlow, grant: Grant) -> State {
    match granted(ui, flow, grant) {
        Some(true) => State::Passed,
        _ if flow.asked.contains(&grant) => State::Waiting,
        Some(false) => State::Failed,
        None if flow.permissions_checking || flow.desktop_checking => State::Checking,
        None => State::Pending,
    }
}

/// Whether macOS wants Hex restarted before `grant` applies.
pub(in crate::rune_ui) fn needs_restart(ui: &Ui, grant: Grant) -> bool {
    let state = match grant {
        Grant::Microphone => ui.platform.permissions.microphone,
        Grant::Accessibility => ui.platform.permissions.accessibility,
        _ => return false,
    };
    state == PermissionState::NeedsRestart
}

/// The checks Finish waits for: the agent answers and can use
/// agent-desktop (for the Claude API, Hex runs agent-desktop itself).
pub(in crate::rune_ui) fn required(backend: AssistantBackend) -> [Check; 2] {
    let desktop = if backend == AssistantBackend::ClaudeApi {
        Check::DesktopRoundTrip
    } else {
        Check::DesktopThroughAgent(backend)
    };
    [Check::Answers(backend), desktop]
}

fn passed(flow: &SetupFlow, check: Check) -> bool {
    flow.checks
        .iter()
        .any(|result| result.check == check && result.state == CheckState::Ok)
}

/// Whether `page`'s checks all work now.
pub(in crate::rune_ui) fn page_done(ui: &Ui, flow: &SetupFlow, page: Page) -> bool {
    let backend = ui.settings.assistant.backend;
    match page {
        Page::Welcome => true,
        Page::Agent => chosen(ui).is_some_and(|found| found.ready),
        Page::Desktop => {
            Grant::HEX
                .iter()
                .all(|grant| grant_state(ui, flow, *grant) == State::Passed)
                && flow
                .desktop
                .as_ref()
                .is_some_and(|found| found.path.is_some())
                && Grant::DESKTOP
                    .iter()
                    .all(|grant| grant_state(ui, flow, *grant) == State::Passed)
        }
        // Off in Settings, web tasks use the person's own browser.
        Page::Browser => flow.browser.done() || !ui.settings.assistant.background_browser,
        Page::Checks => {
            flow.checked_backend == Some(backend)
                && !flow.checks.is_empty()
                && flow.checks.iter().all(CheckResult::ok)
        }
        // Nothing takes the Action shortcut for sure, and a press of it
        // really reached Hex.
        Page::Shortcut => {
            flow.conflicts.as_ref().is_some_and(|found| {
                !found
                    .iter()
                    .any(|(slot, conflict)| *slot == HotkeySlot::Action && conflict.taken)
            }) && matches!(
                probe_state(flow, HotkeySlot::Action),
                Some(ProbeState::Done(Probe::Works(_)))
            )
        }
        Page::Try => flow.tutorial.done(),
    }
}

/// Whether Finish may finish: the chosen agent answers and can use
/// agent-desktop, checked just now.
pub(in crate::rune_ui) fn can_finish(ui: &Ui, flow: &SetupFlow) -> bool {
    let backend = ui.settings.assistant.backend;
    flow.checked_backend == Some(backend)
        && required(backend).iter().all(|check| passed(flow, *check))
}

/// Opens the flow where it was left (Run setup again, Finish setting up),
/// or at its start, and starts looking.
pub(in crate::rune_ui) fn open(ui: &mut Ui) {
    let page = resume::saved_page(ui).unwrap_or(Page::Welcome);
    open_at(ui, page);
}

/// Opens the flow at `page`, looking at everything afresh.
pub(in crate::rune_ui) fn open_at(ui: &mut Ui, page: Page) {
    ui.chats.setup = Some(SetupFlow::default());
    detect(ui);
    recheck(ui);
    browser::read(ui);
    if page != Page::Welcome {
        go(ui, page);
    }
}

/// Looks for the agents again (after an install or a sign-in).
pub(in crate::rune_ui) fn detect(ui: &mut Ui) {
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.detecting = true;
    }
    ui.chats.jobs.run(Job::Detect);
}

/// Reads agent-desktop and Hex's permissions again.
fn recheck(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    flow.checked_at = Some(Instant::now());
    let desktop = !flow.desktop_checking && flow.fixing.is_none();
    let permissions = !flow.permissions_checking;
    flow.desktop_checking |= desktop;
    flow.permissions_checking |= permissions;
    if desktop {
        ui.chats.jobs.run(Job::Desktop);
    }
    if permissions {
        ui.chats.jobs.run(Job::Permissions);
    }
}

/// Moves to `page`; arriving at Checks runs them.
pub(in crate::rune_ui) fn go(ui: &mut Ui, page: Page) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    let left = flow.page;
    flow.page = page;
    flow.fix_error = None;
    if left != page {
        flow.resumed = false;
    }
    resume::remember(ui, page);
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    let stale = flow.checked_backend != Some(ui.settings.assistant.backend);
    if page == Page::Checks && (flow.checks.is_empty() || stale) {
        run_checks(ui);
    }
    if left == Page::Shortcut && page != Page::Shortcut {
        stop_probe(ui);
    }
    if page == Page::Shortcut {
        check_shortcut(ui);
        test_shortcut(ui, HotkeySlot::Action);
    }
    if page == Page::Browser {
        browser::arrive(ui);
    }
    if page == Page::Try
        && let Some(flow) = ui.chats.setup.as_mut()
        && !flow.tutorial.worked(tutorial::Gesture::Here)
    {
        // Gestures count from here on.
        flow.tutorial = tutorial::Tutorial::default();
    }
}

/// The shortcuts the Shortcut page tests: the Action shortcut, which the
/// page waits for, and dictation's beside it.
pub(in crate::rune_ui) const TESTED: [HotkeySlot; 2] = [HotkeySlot::Action, HotkeySlot::Dictation];

/// How long a press test waits for its press.
const PROBE_WAIT: Duration = Duration::from_secs(20);

/// Looks for other apps and macOS features on the tested shortcuts.
pub(in crate::rune_ui) fn check_shortcut(ui: &mut Ui) {
    let settings = ui.settings.clone();
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    flow.conflicts_checked_at = Some(Instant::now());
    if !flow.conflicts_checking {
        flow.conflicts_checking = true;
        ui.chats.jobs.run(Job::Conflicts(Box::new(settings)));
    }
}

/// Asks for a press of `slot`'s shortcut now: the row waits, then shows
/// whether its press and release really reached Hex.
pub(in crate::rune_ui) fn test_shortcut(ui: &mut Ui, slot: HotkeySlot) {
    let live = ui.chats.live();
    stop_probe(ui);
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    flow.probes.retain(|(tested, _)| *tested != slot);
    flow.probes.push((slot, ProbeState::Waiting));
    if live {
        flow.probing = Some((slot, shortcut::start_probe(slot, PROBE_WAIT)));
    }
}

/// Ends a press test still waiting; its row goes back to Test.
fn stop_probe(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if let Some((slot, _)) = flow.probing.take() {
        shortcut::cancel_probe();
        flow.probes.retain(|(tested, _)| *tested != slot);
    }
    flow.probes
        .retain(|(_, state)| *state != ProbeState::Waiting);
}

/// Opens where a conflict is fixed; the page reads them again meanwhile.
pub(in crate::rune_ui) fn clear_conflict(ui: &mut Ui, fix: &ConflictFix) {
    if ui.chats.live() {
        fix.open();
    }
}

/// Another shortcut, in General.
pub(in crate::rune_ui) fn choose_another(ui: &mut Ui, tx: &mut rune::Tx) {
    close(ui);
    ui.select_pane(super::super::Pane::General, tx);
}

/// The press test's state for `slot`.
pub(in crate::rune_ui) fn probe_state(flow: &SetupFlow, slot: HotkeySlot) -> Option<&ProbeState> {
    flow.probes
        .iter()
        .find(|(tested, _)| *tested == slot)
        .map(|(_, state)| state)
}

/// Continue: the next page, once this one works.
pub(in crate::rune_ui) fn advance(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_ref() else {
        return;
    };
    let page = flow.page;
    if !page_done(ui, flow, page) {
        return;
    }
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.later.retain(|later| *later != page);
    }
    if let Some(next) = page.next() {
        go(ui, next);
    }
}

/// Later: the next page, this one left unfinished.
pub(in crate::rune_ui) fn later(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    let page = flow.page;
    if !flow.later.contains(&page) {
        flow.later.push(page);
    }
    match page.next() {
        Some(next) => go(ui, next),
        None => close(ui),
    }
}

pub(in crate::rune_ui) fn back(ui: &mut Ui) {
    if let Some(previous) = ui
        .chats
        .setup
        .as_ref()
        .and_then(|flow| flow.page.previous())
    {
        go(ui, previous);
    }
}

/// Runs the self-tests against the chosen agent.
pub(in crate::rune_ui) fn run_checks(ui: &mut Ui) {
    let backend = ui.settings.assistant.backend;
    let settings = ui.settings.assistant.clone();
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.checks_running {
        return;
    }
    flow.checks_running = true;
    flow.checked_backend = Some(backend);
    flow.checks = Check::for_backend(backend, settings.background_browser)
        .into_iter()
        .map(|check| CheckResult {
            check,
            state: CheckState::Pending,
            detail: String::new(),
            fix: None,
            took: Duration::ZERO,
        })
        .collect();
    ui.chats.jobs.run(Job::SelfTests(Box::new(settings)));
}

/// Asks for a permission: Hex's open their System Settings page with
/// Hex's floating guide, agent-desktop's open its own guide. The row waits
/// until macOS reports it on.
pub(in crate::rune_ui) fn allow(ui: &mut Ui, grant: Grant) {
    if let Some(flow) = ui.chats.setup.as_mut()
        && !flow.asked.contains(&grant)
    {
        flow.asked.push(grant);
    }
    if needs_restart(ui, grant) {
        // One click: Hex relaunches itself and reopens on this page.
        resume::before_restart(ui);
        ui.ask(PlatformRequest::Permission(PermissionAction::Restart));
        return;
    }
    match grant {
        Grant::Microphone => {
            let action = match ui.platform.permissions.microphone {
                PermissionState::NeedsRequest => PermissionAction::RequestMicrophone,
                _ => PermissionAction::ShowGuide(PermissionKind::Microphone),
            };
            ui.ask(PlatformRequest::Permission(action));
        }
        Grant::Accessibility => ui.ask(PlatformRequest::Permission(PermissionAction::ShowGuide(
            PermissionKind::Accessibility,
        ))),
        Grant::ScreenRecording => {
            ui.chats
                .jobs
                .run(Job::Fix(Fix::OpenSettings(Permission::ScreenRecording)));
        }
        Grant::DesktopAccessibility | Grant::DesktopScreenRecording => {
            ui.chats.jobs.run(Job::Fix(Fix::GuidePermissions));
        }
    }
    recheck(ui);
}

/// Does what a failed row offers.
pub(in crate::rune_ui) fn apply(ui: &mut Ui, fix: Fix) {
    match fix {
        Fix::AddApiKey => {
            go(ui, Page::Agent);
            return;
        }
        Fix::Retry => {
            run_checks(ui);
            return;
        }
        Fix::OpenSettings(Permission::Accessibility) => return allow(ui, Grant::Accessibility),
        Fix::OpenSettings(Permission::ScreenRecording) => {
            return allow(ui, Grant::ScreenRecording);
        }
        _ => {}
    }
    if let Some(flow) = ui.chats.setup.as_mut() {
        if flow.fixing.is_some() {
            return;
        }
        flow.fixing = Some(fix);
        flow.fix_error = None;
        flow.fix_log.clear();
        flow.fix_command = None;
    }
    ui.chats.jobs.run(Job::Fix(fix));
}

/// Finish: setup is done, and Chats shows.
pub(in crate::rune_ui) fn finish(ui: &mut Ui, tx: &mut rune::Tx) {
    let Some(flow) = ui.chats.setup.as_ref() else {
        return;
    };
    if !can_finish(ui, flow) {
        return;
    }
    if ui.update(|settings| settings.assistant.setup_done = true) {
        resume::forget(ui);
        close(ui);
        super::show(ui, None, tx);
    }
}

/// Closes the flow. Unfinished, it waits in Chats ("Finish setting up")
/// rather than reopening at launch; continuing the voice first run, that
/// one goes on to "Hex is ready".
pub(in crate::rune_ui) fn close(ui: &mut Ui) {
    stop_probe(ui);
    let merged = ui.chats.setup.as_ref().is_some_and(|flow| flow.merged);
    if ui.chats.setup.is_some() && !ui.settings.assistant.setup_done {
        resume::dismiss(ui);
    }
    ui.chats.setup = None;
    if merged {
        super::super::onboarding_kit::live::release_for_assistant(ui);
    }
}

/// Follows the tutorial's gestures.
pub(super) fn event(ui: &mut Ui, event: &Event) -> bool {
    ui.chats
        .setup
        .as_mut()
        .is_some_and(|flow| flow.tutorial.event(event))
}

/// A job's answer, for the flow.
pub(super) fn receive(ui: &mut Ui, done: super::Done) {
    use super::Done;
    if browser::receive(ui, &done) {
        return;
    }
    let fixed = matches!(done, Done::Fixed(..));
    let browser_installed = matches!(done, Done::Fixed(Fix::InstallAgentBrowser, _));
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    match done {
        Done::Detected(_) => flow.detecting = false,
        Done::Desktop(found) => {
            flow.desktop_checking = false;
            flow.desktop = Some(found);
            forget_granted(ui);
        }
        Done::Permissions(read) => {
            flow.permissions_checking = false;
            flow.permissions = Some(read);
            forget_granted(ui);
        }
        Done::Check(result) => {
            if let Some(slot) = flow
                .checks
                .iter_mut()
                .find(|slot| slot.check == result.check)
            {
                *slot = result;
            }
        }
        Done::AllChecked => flow.checks_running = false,
        Done::FixProgress(fix, line) => {
            if flow.fixing == Some(fix) && !line.trim().is_empty() {
                flow.fix_log.push(line);
                let extra = flow.fix_log.len().saturating_sub(FIX_LOG);
                flow.fix_log.drain(..extra);
            }
        }
        Done::FixCommand(_, command) => flow.fix_command = command,
        Done::Fixed(fix, result) => {
            if flow.fixing == Some(fix) {
                flow.fixing = None;
            }
            match result {
                // What it printed last, and why it stopped.
                Err(error) => {
                    let mut tail = std::mem::take(&mut flow.fix_log);
                    tail.push(error);
                    flow.fix_error = Some(tail.join("\n"));
                }
                Ok(()) => {
                    flow.fix_log.clear();
                    flow.fix_command = None;
                }
            }
        }
        Done::Conflicts(found) => {
            flow.conflicts_checking = false;
            flow.conflicts = Some(found);
        }
        Done::KeySaved(_)
        | Done::Voices(_)
        | Done::RunningApps(_)
        | Done::Browser(..)
        | Done::SignIn(..)
        | Done::BrowserCheck(_)
        | Done::BrowserChecked => {}
    }
    if browser_installed {
        browser::installed(ui);
    }
    if fixed {
        // Installed, started, or signed in: look again.
        detect(ui);
        recheck(ui);
    }
}

/// Granted rows stop waiting.
fn forget_granted(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_ref() else {
        return;
    };
    let granted: Vec<Grant> = flow
        .asked
        .iter()
        .copied()
        .filter(|grant| granted(ui, flow, *grant) == Some(true))
        .collect();
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.asked.retain(|grant| !granted.contains(grant));
    }
}

/// Reads the grants again every moment while a page waits on them, and
/// what the probe heard while the Shortcut page shows.
pub(super) fn poll(ui: &mut Ui, tx: &mut rune::Tx) -> bool {
    let live = ui.chats.live();
    let Some(flow) = ui.chats.setup.as_mut() else {
        return false;
    };
    // A press test's answer.
    if let Some((slot, answer)) = &flow.probing {
        let slot = *slot;
        let done = match answer.try_recv() {
            Ok(found) => Some(found),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Probe::Unavailable(tr("The test stopped").into()))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        };
        if let Some(found) = done {
            flow.probing = None;
            flow.probes.retain(|(tested, _)| *tested != slot);
            flow.probes.push((slot, ProbeState::Done(found)));
            return true;
        }
        tx.wake_after(Duration::from_millis(100));
    }
    // Conflicts are read again while their page shows, so a fix made in
    // System Settings or another app shows.
    if flow.page == Page::Shortcut
        && live
        && flow
            .conflicts_checked_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3))
    {
        check_shortcut(ui);
        tx.wake_after(Duration::from_secs(3));
    }
    // The HUD's double-tap, while the tutorial waits for it.
    if let Some(flow) = ui.chats.setup.as_mut()
        && flow.page == Page::Try
        && !flow.tutorial.tasks
    {
        if flow.tutorial.poll() {
            return true;
        }
        if live {
            tx.wake_after(Duration::from_millis(500));
        }
    }
    let Some(flow) = ui.chats.setup.as_ref() else {
        return false;
    };
    if !ui.chats.live() || flow.page != Page::Desktop {
        return false;
    }
    let due = flow.checked_at.is_none_or(|at| at.elapsed() >= RECHECK);
    if due {
        recheck(ui);
    }
    tx.wake_after(RECHECK);
    false
}

/// The agents a scene finds: Claude Code and Codex ready, OpenCode not
/// running, no API key.
pub(in crate::rune_ui) fn fixture_found() -> Vec<Found> {
    super::BACKENDS
        .iter()
        .map(|backend| {
            let (version, ready, note) = match backend {
                AssistantBackend::ClaudeCode => (Some("2.4.1"), true, None),
                AssistantBackend::Codex => (Some("0.91.0"), false, Some("Not signed in")),
                AssistantBackend::OpenCode => (None, false, Some("Not installed")),
                AssistantBackend::ClaudeApi => (None, false, Some("No API key saved")),
            };
            Found {
                backend: *backend,
                path: version.map(|_| format!("/opt/homebrew/bin/{backend:?}").into()),
                version: version.map(str::to_owned),
                ready,
                note: note.map(str::to_owned),
            }
        })
        .collect()
}

/// The flow for scenes, at `page`, with the checks part way.
pub(in crate::rune_ui) fn fixture(ui: &mut Ui, page: Page, all_passed: bool) {
    ui.chats.settings.found = Some(fixture_found());
    ui.platform.permissions.microphone = PermissionState::Ready;
    ui.platform.permissions.accessibility = PermissionState::Ready;
    let backend = ui.settings.assistant.backend;
    let result = |check: Check, state: CheckState, detail: &str, fix: Option<Fix>| CheckResult {
        check,
        state,
        detail: detail.into(),
        fix,
        took: Duration::ZERO,
    };
    let checks = if all_passed {
        vec![
            result(
                Check::HexPermissions,
                CheckState::Ok,
                "Read Finder's window and the selection",
                None,
            ),
            result(
                Check::Answers(backend),
                CheckState::Ok,
                "Replied “OK” in 2.1 s",
                None,
            ),
            result(
                Check::DesktopPermissions,
                CheckState::Ok,
                "Accessibility and Screen Recording are on",
                None,
            ),
            result(
                Check::DesktopRoundTrip,
                CheckState::Ok,
                "Listed 9 apps and 14 windows",
                None,
            ),
            result(
                Check::DesktopThroughAgent(backend),
                CheckState::Ok,
                "Called desktop_status in 6.4 s",
                None,
            ),
        ]
    } else {
        vec![
            result(
                Check::HexPermissions,
                CheckState::Ok,
                "Read Finder's window and the selection",
                None,
            ),
            result(
                Check::Answers(backend),
                CheckState::Ok,
                "Replied “OK” in 2.1 s",
                None,
            ),
            result(
                Check::DesktopPermissions,
                CheckState::Failed,
                "agent-desktop has no Screen Recording",
                Some(Fix::GuidePermissions),
            ),
            result(Check::DesktopRoundTrip, CheckState::Running, "", None),
            result(
                Check::DesktopThroughAgent(backend),
                CheckState::Pending,
                "",
                None,
            ),
        ]
    };
    ui.chats.setup = Some(SetupFlow {
        page,
        later: if all_passed || page.index() <= Page::Desktop.index() {
            Vec::new()
        } else {
            vec![Page::Desktop]
        },
        desktop: Some(AgentDesktop {
            path: Some("/opt/homebrew/bin/agent-desktop".into()),
            version: Some("0.4.2".into()),
            accessibility: true,
            screen_recording: all_passed,
        }),
        permissions: Some(Permissions {
            accessibility: true,
            screen_recording: all_passed,
        }),
        asked: if all_passed {
            Vec::new()
        } else {
            vec![Grant::ScreenRecording]
        },
        checks,
        checks_running: !all_passed,
        checked_backend: Some(backend),
        conflicts: Some(if all_passed {
            Vec::new()
        } else {
            vec![(
                HotkeySlot::Action,
                Conflict {
                    name: "Siri".into(),
                    description: "Press Right Command twice talks to Siri.".into(),
                    taken: true,
                    fix: ConflictFix::SiriSettings,
                },
            )]
        }),
        probes: if all_passed {
            vec![
                (
                    HotkeySlot::Action,
                    ProbeState::Done(Probe::Works(Duration::from_millis(420))),
                ),
                (
                    HotkeySlot::Dictation,
                    ProbeState::Done(Probe::Works(Duration::from_millis(610))),
                ),
            ]
        } else {
            vec![
                (HotkeySlot::Action, ProbeState::Done(Probe::Swallowed)),
                (HotkeySlot::Dictation, ProbeState::Waiting),
            ]
        },
        tutorial: tutorial::Tutorial::fixture(all_passed),
        ..SetupFlow::default()
    });
    if let Some(flow) = ui.chats.setup.as_mut() {
        browser::fixture(flow, backend, all_passed);
    }
}
