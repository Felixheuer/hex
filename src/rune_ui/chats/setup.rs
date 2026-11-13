//! The assistant's onboarding: a full-window flow in the first run's kit,
//! started when the assistant turns on before it was ever finished, and
//! again from Chats. Page by page it makes sure an agent answers (chosen,
//! installed, signed in), that Agent-Desk (and so Cua Driver) has every permission
//! (each missing one opens its exact System Settings page with Agent-Desk's
//! floating guide, and the page waits until macOS reports the grant), runs
//! the hidden browser for web tasks (`browser`), the live self-tests until
//! they're green, makes sure the Action shortcut and dictation shortcuts
//! are free and that a press of each really reaches Agent-Desk
//! (`recognition::shortcut_probe`), and ends with the Action shortcut's
//! three gestures, each tried for real (`tutorial`).
//!
//! Nothing shows as working until its check ran and saw it work. Continue
//! waits for a page's checks; Later moves on and leaves that page marked
//! unfinished. Finish waits for an agent that answers and can use
//! the desktop tools.

pub(in crate::rune_ui) mod browser;
pub(in crate::rune_ui) mod permissions;
pub(in crate::rune_ui) mod repair;
pub(in crate::rune_ui) mod resume;
pub(in crate::rune_ui) mod tutorial;

mod fixture;
mod grants;
mod probe;
mod receive;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::super::platform::PlatformRequest;

use super::shortcut::{self, Conflict, ConflictFix, Probe};
use super::{Job, Ui};
use crate::app_settings::{AssistantBackend, HotkeySlot};
use crate::assistant::Event;
use crate::assistant::backends::{Check, CheckResult, CheckState, Driver, Fix, Found};
use crate::assistant::computer::{Permission, Permissions};
use crate::desktop::i18n::{tr, tr_fill};
use crate::desktop::onboarding::{PermissionAction, PermissionKind, PermissionState};
pub(in crate::rune_ui) use fixture::{fixture, fixture_found};
use grants::granted;
pub(in crate::rune_ui) use grants::{allow, grant_state, needs_restart};
use probe::stop_probe;
pub(in crate::rune_ui) use probe::{
    TESTED, check_shortcut, choose_another, clear_conflict, probe_state, test_shortcut,
};
#[cfg(test)]
pub(in crate::rune_ui) use receive::gestures_work;
pub(super) use receive::{event, poll, receive};

/// How many of a fix's latest lines it keeps.
const FIX_LOG: usize = 6;

/// How often a page that waits on macOS reads the grants again.
const RECHECK: Duration = Duration::from_millis(1500);

/// The flow's pages, in order: one short path from what Agent-Desk does to
/// the first real question. The optional page (the hidden browser) says so
/// and can be skipped. How freely it acts, the blocked apps, and what it
/// knows about you live in Assistant settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Page {
    /// What Agent-Desk does, on one screen, with what stays on the Mac.
    Welcome,
    /// Every permission in one checklist with its live state: Accessibility,
    /// Screen Recording, the Microphone, Automation, Contacts, Reminders and
    /// the Calendar, and optional Full Disk Access.
    Permissions,
    /// Who answers: chosen, installed, signed in.
    Agent,
    /// Cua Driver, the desktop driver, and the live self-tests proving the
    /// agent answers and can use it.
    Desktop,
    /// Optional: agent-browser, for web tasks in hidden browsers.
    Browser,
    /// Your voice: nothing else takes the Action shortcut, and a press of
    /// it reaches Agent-Desk.
    Shortcut,
    /// The three gestures, tried for real.
    Try,
    /// What's ready and what's left, then Finish.
    Done,
}

impl Page {
    pub(in crate::rune_ui) const ALL: [Page; 8] = [
        Page::Welcome,
        Page::Permissions,
        Page::Agent,
        Page::Desktop,
        Page::Browser,
        Page::Shortcut,
        Page::Try,
        Page::Done,
    ];

    /// Its stable name, as the saved progress keeps it.
    pub(in crate::rune_ui) fn name(self) -> &'static str {
        match self {
            Page::Welcome => "welcome",
            Page::Permissions => "permissions",
            Page::Agent => "agent",
            Page::Desktop => "desktop",
            Page::Browser => "browser",
            Page::Shortcut => "shortcut",
            Page::Try => "try",
            Page::Done => "done",
        }
    }

    /// Whether Finish can do without it.
    pub(in crate::rune_ui) fn optional(self) -> bool {
        self == Page::Browser
    }

    /// Its short name, for the progress line ("Step 2 of 8 · Permissions").
    pub(in crate::rune_ui) fn step_name(self) -> &'static str {
        match self {
            Page::Welcome => tr("Welcome"),
            Page::Permissions => tr("Permissions"),
            Page::Agent => tr("Agent"),
            Page::Desktop => crate::assistant::driver::NAME,
            Page::Browser => tr("Web tasks"),
            Page::Shortcut => tr("Voice"),
            Page::Try => tr("Try it out"),
            Page::Done => tr("Done"),
        }
    }

    /// The pages this OS shows: every one on macOS; Linux and Windows ask
    /// for no permissions here and have no shortcut page, since what takes
    /// a shortcut and the press test read macOS's own settings and event
    /// tap (the Action shortcut's recorder there refuses keys another
    /// shortcut holds).
    pub(in crate::rune_ui) fn shown() -> &'static [Page] {
        const OTHERS: [Page; 6] = [
            Page::Welcome,
            Page::Agent,
            Page::Desktop,
            Page::Browser,
            Page::Try,
            Page::Done,
        ];
        if cfg!(target_os = "macos") {
            &Self::ALL
        } else {
            &OTHERS
        }
    }

    pub(in crate::rune_ui) fn named(name: &str) -> Option<Page> {
        // Progress saved before the pages were merged: each old page's
        // work lives on here now.
        let name = match name {
            "privacy" => "welcome",
            "checks" => "desktop",
            "about" => "try",
            other => other,
        };
        Self::shown()
            .iter()
            .copied()
            .find(|page| page.name() == name)
    }

    fn index(self) -> usize {
        Self::shown()
            .iter()
            .position(|page| *page == self)
            .unwrap_or(0)
    }

    fn next(self) -> Option<Page> {
        Self::shown().get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Page> {
        self.index()
            .checked_sub(1)
            .map(|index| Self::shown()[index])
    }

    /// The next page this OS shows.
    pub(in crate::rune_ui) fn next_shown(self) -> Option<Page> {
        self.next()
    }

    /// The page before, as this OS shows them.
    pub(in crate::rune_ui) fn previous_shown(self) -> Option<Page> {
        self.previous()
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

    /// The grants Agent-Desk asks for itself here: macOS's; Linux and Windows ask
    /// none.
    pub(in crate::rune_ui) fn hex() -> &'static [Grant] {
        if crate::assistant::computer::asks_permissions() {
            &Self::HEX
        } else {
            &[]
        }
    }

    /// Cua Driver's grants here (Agent-Desk's, which it acts with): macOS's;
    /// elsewhere there is no driver.
    pub(in crate::rune_ui) fn desktop() -> &'static [Grant] {
        if crate::assistant::computer::asks_permissions() {
            &Self::DESKTOP
        } else {
            &[]
        }
    }

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
    pub(in crate::rune_ui) desktop: Option<Driver>,
    pub(in crate::rune_ui) desktop_checking: bool,
    /// Agent-Desk's own Accessibility and Screen Recording, read afresh.
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
    permission_checks_pending: bool,
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
    /// The pages continue the voice first run, whose "Agent-Desk is ready" ends
    /// both.
    pub(in crate::rune_ui) merged: bool,
    /// Shown to someone who set Agent-Desk up before the assistant: "New in Agent-Desk".
    pub(in crate::rune_ui) new_in_hex: bool,
    /// Reopened after Agent-Desk restarted part way: the page says so.
    pub(in crate::rune_ui) resumed: bool,
    /// The running fix's latest lines (an installer's output).
    pub(in crate::rune_ui) fix_log: Vec<String>,
    /// The failed fix's command, to run in Terminal instead.
    pub(in crate::rune_ui) fix_command: Option<String>,
    /// Opened by a hold of the Action shortcut before setup finished.
    pub(in crate::rune_ui) from_hold: bool,
    /// Set up before, and back for what's missing now: only these pages.
    pub(in crate::rune_ui) repair: Option<repair::Repair>,
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
            permission_checks_pending: false,
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
            repair: None,
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
        // OpenCode installed but not answering: it needs a key (or a
        // provider sign-in), which the agent picker's own-key panel takes.
        (AssistantBackend::OpenCode, true) => Fix::UseApiKey(AssistantBackend::OpenCode),
        (backend, true) => Fix::SignIn(backend),
    })
}

/// A fix's button, translated.
pub(in crate::rune_ui) fn fix_label(fix: Fix) -> String {
    match fix {
        Fix::Install(backend) => tr_fill("Install {}", super::backend_name(backend)),
        Fix::InstallDriver => tr("Install Cua Driver").into(),
        Fix::UpdateDriver => tr("Update Cua Driver").into(),
        Fix::InstallAgentBrowser => tr("Install agent-browser").into(),
        Fix::InstallLightpanda => tr("Install Lightpanda").into(),
        Fix::SignIn(_) => tr("Sign in…").into(),
        Fix::UseApiKey(_) => tr("Use an API key…").into(),
        Fix::StartOpenCode => tr("Start OpenCode").into(),
        Fix::AddApiKey => tr("Add API key").into(),
        Fix::OpenSettings(Permission::Accessibility) => tr("Open Accessibility…").into(),
        Fix::OpenSettings(Permission::ScreenRecording) => tr("Open Screen Recording…").into(),
        Fix::Retry => tr("Try again").into(),
        Fix::RepairHex => tr("Repair Agent-Desk").into(),
        Fix::RemoveCopies => tr("Remove other copies").into(),
        Fix::RestartHex => tr("Restart Agent-Desk").into(),
        // A fix added after this page was written, in its own words.
        #[allow(unreachable_patterns)]
        _ => fix.label(),
    }
}

/// A self-test's row title, translated.
pub(in crate::rune_ui) fn check_title(check: Check) -> String {
    match check {
        Check::HexPermissions => tr("Agent-Desk can read the screen and selected text").into(),
        Check::Answers(backend) => tr_fill("{} answers", super::backend_name(backend)),
        Check::DesktopPermissions => tr("Cua Driver has Accessibility and Screen Recording").into(),
        Check::DesktopRoundTrip => tr("Cua Driver reads apps and windows").into(),
        Check::DesktopThroughAgent(backend) => {
            tr_fill("{} can use the desktop tools", super::backend_name(backend))
        }
        Check::BrowserRoundTrip => tr("agent-browser opens pages in hidden browsers").into(),
        Check::LightBrowser => tr("Lightpanda reads pages in light browsers").into(),
        Check::BrowserThroughAgent(backend) => {
            tr_fill("{} can use agent-browser", super::backend_name(backend))
        }
    }
}

/// The checks Finish waits for: the agent answers and can use the desktop
/// tools (for the Claude API, Agent-Desk runs them itself).
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
    match page {
        Page::Welcome => true,
        Page::Permissions => {
            Grant::hex()
                .iter()
                .all(|grant| grant_state(ui, flow, *grant) == State::Passed)
                && macos_asks_answered()
        }
        Page::Agent => chosen(ui).is_some_and(|found| found.ready),
        // Cua Driver is there with its grants, and every self-test passed.
        Page::Desktop => {
            (!cfg!(target_os = "macos")
                || flow
                    .desktop
                    .as_ref()
                    .is_some_and(|found| found.path.is_some()))
                && Grant::desktop()
                    .iter()
                    .all(|grant| grant_state(ui, flow, *grant) == State::Passed)
                && checks_passed(ui, flow)
        }
        // Off in Settings, web tasks use the person's own browser.
        Page::Browser => flow.browser.done() || !ui.settings.assistant.background_browser,
        // Nothing takes the Action shortcut for sure, and a press of it
        // really reached Agent-Desk.
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
        Page::Done => can_finish(ui, flow),
    }
}

/// Every self-test against the chosen agent passed (the browser's optional
/// ones may not).
pub(in crate::rune_ui) fn checks_passed(ui: &Ui, flow: &SetupFlow) -> bool {
    flow.checked_backend == Some(ui.settings.assistant.backend)
        && !flow.checks.is_empty()
        && flow.checks.iter().all(browser::passed_or_optional)
}

/// macOS's own prompts on the permissions page (Automation for System
/// Events and Finder; Contacts, Reminders, and the Calendar) were each
/// answered, yes or no: a refusal is the person's answer, and only one
/// never asked holds the page. Not read yet counts as answered, so a
/// slow read never holds it either.
fn macos_asks_answered() -> bool {
    use crate::desktop::automation::{self, AutomationState};
    use crate::desktop::data_access;
    !cfg!(target_os = "macos")
        || (automation::last() != AutomationState::NotAsked
            && data_access::all_answered(data_access::last))
}

/// Whether Finish may finish: the chosen agent answers and can use the
/// desktop tools, checked just now.
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
    super::displays::read(ui);
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

/// Reads Cua Driver and Agent-Desk's permissions again.
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
    // Automation for System Events and Finder, and the person's data, read
    // where they're asked for.
    if cfg!(target_os = "macos") && ui.chats.live() {
        crate::desktop::automation::refresh_async();
        crate::desktop::data_access::refresh_async();
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
    if matches!(page, Page::Desktop | Page::Done) && (flow.checks.is_empty() || stale) {
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
    let Some(flow) = ui.chats.setup.as_ref() else {
        return;
    };
    match repair::next_page(flow, page) {
        Some(next) => go(ui, next),
        // The guide's last page: setup is whole again.
        None if flow.repair.is_some() => repair::finish(ui),
        None => {}
    }
}

/// Later: the next page, this one left unfinished.
pub(in crate::rune_ui) fn later(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if flow.repair.is_some() {
        // The guide comes back when Agent-Desk next starts.
        repair::snooze(ui);
        return;
    }
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
        .and_then(|flow| repair::previous_page(flow, flow.page))
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
    let checks = flow.checks.iter().map(|result| result.check).collect();
    ui.chats
        .jobs
        .run(Job::SelfTests(Box::new(settings), checks));
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
/// one goes on to "Agent-Desk is ready".
pub(in crate::rune_ui) fn close(ui: &mut Ui) {
    stop_probe(ui);
    crate::assistant::set_trying(false);
    let merged = ui.chats.setup.as_ref().is_some_and(|flow| flow.merged);
    if ui
        .chats
        .setup
        .as_ref()
        .is_some_and(|flow| flow.repair.is_some())
    {
        // Closed with something still missing: not again until Agent-Desk next
        // starts (a hold still opens it).
        ui.chats.repair.snoozed = true;
    }
    if ui.chats.setup.is_some() && !ui.settings.assistant.setup_done {
        resume::dismiss(ui);
    }
    ui.chats.setup = None;
    if merged {
        super::super::onboarding_kit::live::release_for_assistant(ui);
    }
}
