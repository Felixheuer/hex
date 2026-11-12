//! The assistant's onboarding: a full-window flow in the first run's kit,
//! started when Labs › Assistant turns on before it was ever finished, and
//! again from Chats. Page by page it makes sure an agent answers (chosen,
//! installed, signed in), that Hex and agent-desktop have every permission
//! (each missing one opens its exact System Settings page with Hex's
//! floating guide, and the page waits until macOS reports the grant), runs
//! the live self-tests until they're green, makes sure the Action shortcut
//! is free and that Hex really receives it (hold and double-tap), and ends
//! with one real spoken question through it.
//!
//! Nothing shows as working until its check ran and saw it work. Continue
//! waits for a page's checks; Later moves on and leaves that page marked
//! unfinished. Finish waits for an agent that answers and can use
//! agent-desktop.

use std::time::{Duration, Instant};

use super::super::platform::PlatformRequest;
use super::{Job, Ui};
use super::shortcut_stub::{self as shortcut, Conflict, ConflictFix, Heard};
use crate::app_settings::{AssistantBackend, HotkeySlot};
use crate::assistant::backends::{AgentDesktop, Check, CheckResult, CheckState, Fix, Found};
use crate::assistant::computer::{Permission, Permissions};
use crate::assistant::{ConversationId, Event, Origin};
use crate::desktop::i18n::{tr, tr_fill};
use crate::desktop::onboarding::{PermissionAction, PermissionKind, PermissionState};

/// How often a page that waits on macOS reads the grants again.
const RECHECK: Duration = Duration::from_millis(1500);

/// The flow's pages, in order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Page {
    Welcome,
    Agent,
    Permissions,
    Desktop,
    Checks,
    /// Nothing else takes the Action shortcut, and Hex receives it.
    Shortcut,
    Try,
}

impl Page {
    pub(in crate::rune_ui) const ALL: [Page; 7] = [
        Page::Welcome,
        Page::Agent,
        Page::Permissions,
        Page::Desktop,
        Page::Checks,
        Page::Shortcut,
        Page::Try,
    ];

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
    /// What else takes the Action shortcut, once checked (or why it
    /// couldn't be).
    pub(in crate::rune_ui) conflicts: Option<Result<Vec<Conflict>, String>>,
    /// What the live probe heard of the Action shortcut.
    pub(in crate::rune_ui) heard_keys: Heard,
    probing: bool,
    /// The try-it question: heard, then answered.
    pub(in crate::rune_ui) asked_question: Option<ConversationId>,
    pub(in crate::rune_ui) heard: Option<String>,
    pub(in crate::rune_ui) answer: Option<Result<String, String>>,
    checked_at: Option<Instant>,
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
            heard_keys: Heard::default(),
            probing: false,
            asked_question: None,
            heard: None,
            answer: None,
            checked_at: None,
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
        Fix::SignIn(_) => tr("Sign in…").into(),
        Fix::StartOpenCode => tr("Start OpenCode").into(),
        Fix::AddApiKey => tr("Add API key").into(),
        Fix::GuidePermissions => tr("Grant permissions…").into(),
        Fix::OpenSettings(Permission::Accessibility) => tr("Open Accessibility…").into(),
        Fix::OpenSettings(Permission::ScreenRecording) => tr("Open Screen Recording…").into(),
        Fix::Retry => tr("Try again").into(),
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
        Page::Permissions => Grant::HEX
            .iter()
            .all(|grant| grant_state(ui, flow, *grant) == State::Passed),
        Page::Desktop => {
            flow.desktop.as_ref().is_some_and(|found| found.path.is_some())
                && Grant::DESKTOP
                    .iter()
                    .all(|grant| grant_state(ui, flow, *grant) == State::Passed)
        }
        Page::Checks => {
            flow.checked_backend == Some(backend)
                && !flow.checks.is_empty()
                && flow.checks.iter().all(CheckResult::ok)
        }
        Page::Shortcut => {
            matches!(&flow.conflicts, Some(Ok(found)) if found.is_empty())
                && flow.heard_keys.hold
                && flow.heard_keys.double_tap
        }
        Page::Try => matches!(flow.answer, Some(Ok(_))),
    }
}

/// Whether Finish may finish: the chosen agent answers and can use
/// agent-desktop, checked just now.
pub(in crate::rune_ui) fn can_finish(ui: &Ui, flow: &SetupFlow) -> bool {
    let backend = ui.settings.assistant.backend;
    flow.checked_backend == Some(backend)
        && required(backend).iter().all(|check| passed(flow, *check))
}

/// Opens the flow at its start, and starts looking.
pub(in crate::rune_ui) fn open(ui: &mut Ui) {
    ui.chats.setup = Some(SetupFlow::default());
    detect(ui);
    recheck(ui);
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
    let stale = flow.checked_backend != Some(ui.settings.assistant.backend);
    if page == Page::Checks && (flow.checks.is_empty() || stale) {
        run_checks(ui);
    }
    if left == Page::Shortcut && page != Page::Shortcut {
        stop_probe(ui);
    }
    if page == Page::Shortcut {
        check_shortcut(ui);
    }
}

/// Looks for other apps on the Action shortcut, and starts listening for
/// it.
pub(in crate::rune_ui) fn check_shortcut(ui: &mut Ui) {
    let settings = ui.settings.clone();
    let live = ui.chats.live();
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    flow.conflicts = None;
    flow.heard_keys = Heard::default();
    ui.chats.jobs.run(Job::Conflicts(Box::new(settings)));
    if live {
        shortcut::start_probe(HotkeySlot::Action);
        if let Some(flow) = ui.chats.setup.as_mut() {
            flow.probing = true;
        }
    }
}

fn stop_probe(ui: &mut Ui) {
    if let Some(flow) = ui.chats.setup.as_mut()
        && std::mem::take(&mut flow.probing)
    {
        shortcut::stop_probe(HotkeySlot::Action);
    }
}

/// Clears a conflict on the Action shortcut.
pub(in crate::rune_ui) fn clear_conflict(ui: &mut Ui, fix: ConflictFix, tx: &mut rune::Tx) {
    match fix {
        ConflictFix::ChooseAnother => {
            close(ui);
            ui.select_pane(super::super::Pane::General, tx);
        }
        fix => ui.chats.jobs.run(Job::ClearConflict(fix)),
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
    if let Some(previous) = ui.chats.setup.as_ref().and_then(|flow| flow.page.previous()) {
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
    flow.checks = Check::for_backend(backend)
        .into_iter()
        .map(|check| CheckResult {
            check,
            state: CheckState::Pending,
            detail: String::new(),
            fix: None,
            took: Duration::ZERO,
        })
        .collect();
    ui.chats.jobs.run(Job::SelfTests(settings));
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
        close(ui);
        super::show(ui, None, tx);
    }
}

pub(in crate::rune_ui) fn close(ui: &mut Ui) {
    stop_probe(ui);
    ui.chats.setup = None;
}

/// Follows the try-it question.
pub(super) fn event(ui: &mut Ui, event: &Event) -> bool {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return false;
    };
    match event {
        Event::Queued {
            conversation,
            origin: Origin::ActionShortcut,
            question,
        }
        | Event::Started {
            conversation,
            origin: Origin::ActionShortcut,
            question,
        } => {
            flow.asked_question = Some(conversation.clone());
            flow.heard = Some(question.clone());
            flow.answer = None;
        }
        Event::Answer {
            conversation,
            origin: Origin::ActionShortcut,
            text,
        } if flow
            .asked_question
            .as_ref()
            .is_none_or(|asked| asked == conversation) =>
        {
            flow.asked_question = None;
            flow.answer = Some(Ok(text.trim().to_owned()));
        }
        Event::Failed {
            conversation,
            message,
        } if flow.asked_question.as_ref() == Some(conversation) => {
            flow.asked_question = None;
            flow.answer = Some(Err(message.clone()));
        }
        Event::Cancelled { conversation } if flow.asked_question.as_ref() == Some(conversation) => {
            flow.asked_question = None;
            flow.heard = None;
        }
        _ => return false,
    }
    true
}

/// A job's answer, for the flow.
pub(super) fn receive(ui: &mut Ui, done: super::Done) {
    use super::Done;
    let fixed = matches!(done, Done::Fixed(..));
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
            if let Some(slot) = flow.checks.iter_mut().find(|slot| slot.check == result.check) {
                *slot = result;
            }
        }
        Done::ChecksDone => flow.checks_running = false,
        Done::Fixed(fix, result) => {
            if flow.fixing == Some(fix) {
                flow.fixing = None;
            }
            if let Err(error) = result {
                flow.fix_error = Some(error);
            }
        }
        Done::Conflicts(found) => flow.conflicts = Some(found),
        Done::ConflictCleared(Err(error)) => flow.fix_error = Some(error),
        Done::ConflictCleared(Ok(())) => {
            check_shortcut(ui);
            return;
        }
        Done::KeySaved(_) | Done::Voices(_) => {}
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
    let Some(flow) = ui.chats.setup.as_mut() else {
        return false;
    };
    if flow.probing {
        let heard = shortcut::probe(HotkeySlot::Action);
        let changed = heard != flow.heard_keys;
        flow.heard_keys.hold |= heard.hold;
        flow.heard_keys.double_tap |= heard.double_tap;
        tx.wake_after(Duration::from_millis(100));
        if changed {
            return true;
        }
    }
    let Some(flow) = ui.chats.setup.as_ref() else {
        return false;
    };
    if !ui.chats.live() || !matches!(flow.page, Page::Permissions | Page::Desktop) {
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
            result(Check::HexPermissions, CheckState::Ok, "Read Finder's window and the selection", None),
            result(Check::Answers(backend), CheckState::Ok, "Replied “OK” in 2.1 s", None),
            result(Check::DesktopPermissions, CheckState::Ok, "Accessibility and Screen Recording are on", None),
            result(Check::DesktopRoundTrip, CheckState::Ok, "Listed 9 apps and 14 windows", None),
            result(Check::DesktopThroughAgent(backend), CheckState::Ok, "Called desktop_status in 6.4 s", None),
        ]
    } else {
        vec![
            result(Check::HexPermissions, CheckState::Ok, "Read Finder's window and the selection", None),
            result(Check::Answers(backend), CheckState::Ok, "Replied “OK” in 2.1 s", None),
            result(
                Check::DesktopPermissions,
                CheckState::Failed,
                "agent-desktop has no Screen Recording",
                Some(Fix::GuidePermissions),
            ),
            result(Check::DesktopRoundTrip, CheckState::Running, "", None),
            result(Check::DesktopThroughAgent(backend), CheckState::Pending, "", None),
        ]
    };
    ui.chats.setup = Some(SetupFlow {
        page,
        later: if all_passed || page.index() <= Page::Desktop.index() { Vec::new() } else { vec![Page::Desktop] },
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
            Ok(Vec::new())
        } else {
            Ok(vec![Conflict {
                owner: "Siri".into(),
                detail: "Press right ⌘ twice talks to Siri".into(),
                fix: Some(ConflictFix::OpenSettings(
                    "x-apple.systempreferences:com.apple.Siri-Settings.extension".into(),
                )),
            }])
        }),
        heard_keys: Heard {
            hold: true,
            double_tap: all_passed,
        },
        heard: all_passed.then(|| "What time is it in Tokyo?".into()),
        answer: all_passed.then(|| Ok("It's 9:41 PM in Tokyo.".into())),
        ..SetupFlow::default()
    });
}
