//! The assistant's setup: shown when Labs › Assistant turns on before it was
//! ever finished, and again from Chats. Each step is checked live: the
//! agents found on this Mac (one chosen to answer), agent-desktop, the
//! permissions Hex and agent-desktop need, self-tests that send real
//! requests through them, and one spoken question through the Action
//! shortcut. A step shows ✓ only after its check ran and saw it work;
//! Finish waits for every step that applies.

use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, bail, eyre};

use super::super::platform::PlatformRequest;
use super::{BACKENDS, Job, Ui, backend_name};
use crate::app_settings::AssistantBackend;
use crate::assistant::backends::{self, AgentDesktop, Found};
use crate::assistant::computer::Permissions;
use crate::assistant::{ConversationId, Event, Origin};
use crate::desktop::i18n::{tr, tr_fill};
use crate::desktop::onboarding::{PermissionAction, PermissionKind, PermissionState};


/// How often the sheet reads agent-desktop and the permissions again while
/// it shows, so a grant in System Settings turns green on its own.
const RECHECK: Duration = Duration::from_secs(2);

/// Where a check stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::rune_ui) enum State {
    /// Not checked yet, or waiting on an earlier step.
    #[default]
    Pending,
    Checking,
    /// Checked, and it works.
    Passed,
    /// Checked, and it doesn't.
    Failed,
    /// Doesn't apply with these settings.
    Skipped,
}

/// A check's state and what it saw, for a person.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::rune_ui) struct Status {
    pub(in crate::rune_ui) state: State,
    pub(in crate::rune_ui) detail: String,
}

impl Status {
    fn new(state: State, detail: impl Into<String>) -> Self {
        Self {
            state,
            detail: detail.into(),
        }
    }
    pub(in crate::rune_ui) fn pending(detail: impl Into<String>) -> Self {
        Self::new(State::Pending, detail)
    }
    pub(in crate::rune_ui) fn checking(detail: impl Into<String>) -> Self {
        Self::new(State::Checking, detail)
    }
    pub(in crate::rune_ui) fn passed(detail: impl Into<String>) -> Self {
        Self::new(State::Passed, detail)
    }
    pub(in crate::rune_ui) fn failed(detail: impl Into<String>) -> Self {
        Self::new(State::Failed, detail)
    }
    pub(in crate::rune_ui) fn skipped(detail: impl Into<String>) -> Self {
        Self::new(State::Skipped, detail)
    }
    /// Whether Finish may go ahead past it.
    fn settled(&self) -> bool {
        matches!(self.state, State::Passed | State::Skipped)
    }
}

/// One self-test. Each sends a real request through the pieces a task uses;
/// `run` blocks (it runs on its own thread) and returns what it saw, or why
/// it failed, for a person.
pub(in crate::rune_ui) struct SetupCheck {
    pub(in crate::rune_ui) id: &'static str,
    /// Shown through `tr`.
    pub(in crate::rune_ui) title: &'static str,
    /// It needs agent-desktop, so it's skipped while Operate apps is off.
    pub(in crate::rune_ui) needs_desktop: bool,
    pub(in crate::rune_ui) run: fn(AssistantBackend) -> Result<String>,
}

/// The self-tests, in the order they run. Until the hands' checks land in
/// `backends`, each body checks what it can and otherwise fails, so nothing
/// shows ✓ untested.
pub(in crate::rune_ui) const SELF_TESTS: [SetupCheck; 4] = [
    SetupCheck {
        id: "backend-answers",
        title: "The agent answers",
        needs_desktop: false,
        run: backend_answers,
    },
    SetupCheck {
        id: "desktop-runs",
        title: "agent-desktop runs from Hex",
        needs_desktop: true,
        run: desktop_runs,
    },
    SetupCheck {
        id: "desktop-reads",
        title: "agent-desktop reads the screen",
        needs_desktop: true,
        run: desktop_reads,
    },
    SetupCheck {
        id: "agent-reaches-desktop",
        title: "The agent reaches agent-desktop",
        needs_desktop: true,
        run: agent_reaches_desktop,
    },
];

fn not_yet() -> color_eyre::Report {
    eyre!(tr("This check isn't in this build yet"))
}

/// The chosen agent answers a one-word prompt with "OK".
/// Integration: `backends::check_backend(backend)`.
fn backend_answers(backend: AssistantBackend) -> Result<String> {
    if backend != AssistantBackend::ClaudeApi && backends::executable(backend).is_none() {
        bail!(tr_fill("{} isn't installed", backend_name(backend)));
    }
    Err(not_yet())
}

/// agent-desktop starts from inside Hex (its environment, its sandbox).
/// Integration: `backends::check_agent_desktop()`.
fn desktop_runs(_: AssistantBackend) -> Result<String> {
    let path = backends::agent_desktop()
        .path
        .ok_or_else(|| eyre!(tr("agent-desktop isn't installed")))?;
    let output = std::process::Command::new(&path).arg("--version").output()?;
    let said = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    if !output.status.success() {
        bail!(said(&output.stderr));
    }
    Ok(said(&output.stdout))
}

/// A read-only round trip: agent-desktop lists the frontmost app's window.
/// Integration: `backends::check_desktop_read()`.
fn desktop_reads(_: AssistantBackend) -> Result<String> {
    Err(not_yet())
}

/// The agent calls agent-desktop over MCP and reports what it saw.
/// Integration: `backends::check_agent_mcp(backend)`.
fn agent_reaches_desktop(_: AssistantBackend) -> Result<String> {
    Err(not_yet())
}

/// The sheet's checks as they stand.
#[derive(Default)]
pub(in crate::rune_ui) struct SetupSheet {
    pub(in crate::rune_ui) detecting: bool,
    pub(in crate::rune_ui) desktop: Option<AgentDesktop>,
    pub(in crate::rune_ui) desktop_checking: bool,
    pub(in crate::rune_ui) permissions: Option<Permissions>,
    pub(in crate::rune_ui) permissions_checking: bool,
    pub(in crate::rune_ui) installing: bool,
    pub(in crate::rune_ui) install_error: Option<String>,
    pub(in crate::rune_ui) tests: [Status; 4],
    pub(in crate::rune_ui) tests_ran: bool,
    pub(in crate::rune_ui) shortcut: Status,
    /// The quick question the shortcut step is following.
    pub(in crate::rune_ui) asked: Option<ConversationId>,
    checked_at: Option<Instant>,
}

impl SetupSheet {
    pub(super) fn new() -> Self {
        Self {
            shortcut: Status::pending(tr("Waiting for your question")),
            ..Self::default()
        }
    }

    fn tests_running(&self) -> bool {
        self.tests.iter().any(|test| test.state == State::Checking)
    }
}

/// Which permission a row asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Grant {
    Microphone,
    Accessibility,
    ScreenRecording,
    DesktopAccessibility,
    DesktopScreenRecording,
}

impl Grant {
    pub(in crate::rune_ui) const ALL: [Grant; 5] = [
        Grant::Microphone,
        Grant::Accessibility,
        Grant::ScreenRecording,
        Grant::DesktopAccessibility,
        Grant::DesktopScreenRecording,
    ];

    pub(in crate::rune_ui) fn key(self) -> &'static str {
        match self {
            Grant::Microphone => "grant-microphone",
            Grant::Accessibility => "grant-accessibility",
            Grant::ScreenRecording => "grant-screen-recording",
            Grant::DesktopAccessibility => "grant-desktop-accessibility",
            Grant::DesktopScreenRecording => "grant-desktop-screen-recording",
        }
    }

    pub(in crate::rune_ui) fn name(self) -> &'static str {
        match self {
            Grant::Microphone => tr("Microphone"),
            Grant::Accessibility | Grant::DesktopAccessibility => tr("Accessibility"),
            Grant::ScreenRecording | Grant::DesktopScreenRecording => tr("Screen Recording"),
        }
    }

    /// Whose permission it is.
    pub(in crate::rune_ui) fn owner(self) -> &'static str {
        match self {
            Grant::Microphone | Grant::Accessibility | Grant::ScreenRecording => "Hex",
            Grant::DesktopAccessibility | Grant::DesktopScreenRecording => "agent-desktop",
        }
    }

    /// Hex's own permission kind, for its guide.
    pub(in crate::rune_ui) fn kind(self) -> Option<PermissionKind> {
        match self {
            Grant::Microphone => Some(PermissionKind::Microphone),
            Grant::Accessibility => Some(PermissionKind::Accessibility),
            Grant::ScreenRecording => Some(PermissionKind::ScreenRecording),
            Grant::DesktopAccessibility | Grant::DesktopScreenRecording => None,
        }
    }
}

/// Every step's status, from the sheet's checks and the live settings.
pub(in crate::rune_ui) struct Report {
    pub(in crate::rune_ui) agents: Status,
    pub(in crate::rune_ui) desktop: Status,
    pub(in crate::rune_ui) grants: Vec<(Grant, Status)>,
    pub(in crate::rune_ui) tests: Vec<Status>,
    pub(in crate::rune_ui) shortcut: Status,
}

impl Report {
    /// Whether every step that applies works.
    pub(in crate::rune_ui) fn ready(&self) -> bool {
        self.agents.settled()
            && self.desktop.settled()
            && self.grants.iter().all(|(_, status)| status.settled())
            && self.tests.iter().all(Status::settled)
            && self.shortcut.settled()
    }

    /// How many of the five steps work.
    pub(in crate::rune_ui) fn done(&self) -> usize {
        [
            self.agents.settled(),
            self.desktop.settled(),
            self.grants.iter().all(|(_, status)| status.settled()),
            self.tests.iter().all(Status::settled),
            self.shortcut.settled(),
        ]
        .into_iter()
        .filter(|done| *done)
        .count()
    }

    pub(in crate::rune_ui) fn permissions(&self) -> Status {
        combine(self.grants.iter().map(|(_, status)| status))
    }

    pub(in crate::rune_ui) fn checks(&self) -> Status {
        combine(self.tests.iter())
    }
}

/// A step's status from its rows': failed if any failed, checking if any
/// is, passed once all are settled, else pending.
fn combine<'a>(rows: impl Iterator<Item = &'a Status>) -> Status {
    let mut rows = rows.peekable();
    if rows.peek().is_none() {
        return Status::default();
    }
    let all: Vec<&Status> = rows.collect();
    let state = if all.iter().any(|row| row.state == State::Failed) {
        State::Failed
    } else if all.iter().any(|row| row.state == State::Checking) {
        State::Checking
    } else if all.iter().all(|row| row.settled()) {
        if all.iter().all(|row| row.state == State::Skipped) {
            State::Skipped
        } else {
            State::Passed
        }
    } else {
        State::Pending
    };
    Status::new(state, String::new())
}

/// The agent chosen in `found`, if it was looked for.
pub(in crate::rune_ui) fn chosen(ui: &Ui) -> Option<&Found> {
    let backend = ui.settings.assistant.backend;
    ui.chats
        .settings
        .found
        .as_ref()?
        .iter()
        .find(|found| found.backend == backend)
}

fn hex_permission(state: PermissionState) -> Status {
    match state {
        PermissionState::Ready => Status::passed(tr("Allowed")),
        PermissionState::NeedsRestart => Status::failed(tr("Turns on when Hex restarts")),
        PermissionState::NeedsRequest | PermissionState::NeedsSettings => {
            Status::failed(tr("Not allowed yet"))
        }
    }
}

pub(in crate::rune_ui) fn report(ui: &Ui, sheet: &SetupSheet) -> Report {
    let assistant = &ui.settings.assistant;
    let agents = match (&ui.chats.settings.found, chosen(ui)) {
        (_, _) if sheet.detecting => Status::checking(tr("Looking for agents…")),
        (None, _) => Status::pending(""),
        (Some(_), Some(found)) if found.ready => Status::passed(tr_fill(
            "{} answers",
            &match &found.version {
                Some(version) => format!("{} {version}", backend_name(found.backend)),
                None => backend_name(found.backend).to_owned(),
            },
        )),
        (Some(_), Some(found)) => Status::failed(
            found
                .note
                .clone()
                .unwrap_or_else(|| tr_fill("{} isn't ready", backend_name(found.backend))),
        ),
        (Some(_), None) => Status::failed(tr_fill(
            "{} isn't on this Mac",
            backend_name(assistant.backend),
        )),
    };
    let desktop = if !assistant.control_desktop {
        Status::skipped(tr("Not needed while Operate apps is off"))
    } else if sheet.installing {
        Status::checking(tr("Installing with Homebrew…"))
    } else if let Some(error) = &sheet.install_error {
        Status::failed(error.clone())
    } else {
        match &sheet.desktop {
            None if sheet.desktop_checking => Status::checking(tr("Looking for agent-desktop…")),
            None => Status::pending(""),
            Some(found) => match (&found.path, &found.version) {
                (Some(_), Some(version)) => Status::passed(tr_fill("Version {}", version)),
                (Some(path), None) => Status::passed(path.display().to_string()),
                (None, _) => Status::failed(tr("Not installed")),
            },
        }
    };
    let grants = Grant::ALL
        .into_iter()
        .map(|grant| (grant, grant_status(ui, sheet, grant)))
        .collect();
    let tests = SELF_TESTS
        .iter()
        .zip(&sheet.tests)
        .map(|(check, status)| {
            if check.needs_desktop && !assistant.control_desktop {
                Status::skipped(tr("Not needed while Operate apps is off"))
            } else if status.state == State::Pending && !sheet.tests_ran {
                Status::pending("")
            } else {
                status.clone()
            }
        })
        .collect();
    let shortcut = if assistant.action_hotkey.is_none() {
        Status::failed(tr("No Action shortcut is set. Choose one in General"))
    } else {
        sheet.shortcut.clone()
    };
    Report {
        agents,
        desktop,
        grants,
        tests,
        shortcut,
    }
}

fn grant_status(ui: &Ui, sheet: &SetupSheet, grant: Grant) -> Status {
    let assistant = &ui.settings.assistant;
    let permissions = &ui.platform.permissions;
    match grant {
        Grant::Microphone => hex_permission(permissions.microphone),
        Grant::Accessibility => hex_permission(permissions.accessibility),
        Grant::ScreenRecording => {
            if !assistant.include_screenshot {
                return Status::skipped(tr("Not needed without screenshots"));
            }
            match sheet.permissions {
                None if sheet.permissions_checking => Status::checking(""),
                None => Status::pending(""),
                Some(read) if read.screen_recording => Status::passed(tr("Allowed")),
                Some(_) => Status::failed(tr("Not allowed yet")),
            }
        }
        Grant::DesktopAccessibility | Grant::DesktopScreenRecording => {
            if !assistant.control_desktop {
                return Status::skipped(tr("Not needed while Operate apps is off"));
            }
            match &sheet.desktop {
                None if sheet.desktop_checking => Status::checking(""),
                None => Status::pending(""),
                Some(found) if found.path.is_none() => {
                    Status::pending(tr("Install agent-desktop first"))
                }
                Some(found) => {
                    let allowed = if grant == Grant::DesktopAccessibility {
                        found.accessibility
                    } else {
                        found.screen_recording
                    };
                    if allowed {
                        Status::passed(tr("Allowed"))
                    } else {
                        Status::failed(tr("Not allowed yet"))
                    }
                }
            }
        }
    }
}

/// Starts the sheet's first checks.
pub(super) fn start(ui: &mut Ui) {
    detect(ui);
    recheck(ui);
}

/// Looks for the agents again.
pub(in crate::rune_ui) fn detect(ui: &mut Ui) {
    if let Some(sheet) = ui.chats.setup.as_mut() {
        sheet.detecting = true;
    }
    ui.chats.jobs.run(Job::Detect);
}

/// Reads agent-desktop and Hex's permissions again.
fn recheck(ui: &mut Ui) {
    let Some(sheet) = ui.chats.setup.as_mut() else {
        return;
    };
    sheet.checked_at = Some(Instant::now());
    if !sheet.desktop_checking && !sheet.installing {
        sheet.desktop_checking = true;
        ui.chats.jobs.run(Job::Desktop);
    }
    let Some(sheet) = ui.chats.setup.as_mut() else {
        return;
    };
    if !sheet.permissions_checking {
        sheet.permissions_checking = true;
        ui.chats.jobs.run(Job::Permissions);
    }
}

/// Installs agent-desktop with Homebrew.
pub(in crate::rune_ui) fn install(ui: &mut Ui) {
    let Some(sheet) = ui.chats.setup.as_mut() else {
        return;
    };
    if sheet.installing {
        return;
    }
    sheet.installing = true;
    sheet.install_error = None;
    ui.chats.jobs.run(Job::Install);
}

/// Runs the self-tests against the chosen agent.
pub(in crate::rune_ui) fn run_tests(ui: &mut Ui) {
    let control_desktop = ui.settings.assistant.control_desktop;
    let backend = ui.settings.assistant.backend;
    let Some(sheet) = ui.chats.setup.as_mut() else {
        return;
    };
    if sheet.tests_running() {
        return;
    }
    sheet.tests_ran = true;
    for (status, check) in sheet.tests.iter_mut().zip(&SELF_TESTS) {
        *status = if check.needs_desktop && !control_desktop {
            Status::skipped("")
        } else {
            Status::checking("")
        };
    }
    ui.chats.jobs.run(Job::SelfTests {
        backend,
        desktop: control_desktop,
    });
}

/// Asks for a permission: Hex's through its guide, agent-desktop's through
/// its own.
pub(in crate::rune_ui) fn allow(ui: &mut Ui, grant: Grant) {
    let state = match grant {
        Grant::Microphone => ui.platform.permissions.microphone,
        Grant::Accessibility => ui.platform.permissions.accessibility,
        _ => PermissionState::NeedsSettings,
    };
    let action = match (grant.kind(), state) {
        (None, _) => {
            ui.chats.jobs.run(Job::Guide);
            return;
        }
        (Some(_), PermissionState::NeedsRestart) => PermissionAction::Restart,
        (Some(PermissionKind::Microphone), PermissionState::NeedsRequest) => {
            PermissionAction::RequestMicrophone
        }
        (Some(kind), _) => PermissionAction::ShowGuide(kind),
    };
    ui.ask(PlatformRequest::Permission(action));
}

/// Finish: setup is done, and Chats shows.
pub(in crate::rune_ui) fn finish(ui: &mut Ui, tx: &mut rune::Tx) {
    let Some(sheet) = ui.chats.setup.as_ref() else {
        return;
    };
    if !report(ui, sheet).ready() {
        return;
    }
    if ui.update(|settings| settings.assistant.setup_done = true) {
        ui.chats.setup = None;
        super::show(ui, None, tx);
    }
}

pub(in crate::rune_ui) fn close(ui: &mut Ui) {
    ui.chats.setup = None;
}

/// Follows the quick question the shortcut step waits for.
pub(super) fn event(ui: &mut Ui, event: &Event) -> bool {
    let Some(sheet) = ui.chats.setup.as_mut() else {
        return false;
    };
    match event {
        Event::Queued {
            conversation,
            origin: Origin::ActionShortcut,
            ..
        }
        | Event::Started {
            conversation,
            origin: Origin::ActionShortcut,
            ..
        } => {
            sheet.asked = Some(conversation.clone());
            sheet.shortcut = Status::checking(tr("Heard you. Waiting for the answer…"));
        }
        Event::Answer {
            conversation,
            origin: Origin::ActionShortcut,
            text,
        } if sheet.asked.as_ref().is_none_or(|asked| asked == conversation) => {
            sheet.asked = None;
            let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default();
            let short: String = line.chars().take(90).collect();
            sheet.shortcut = Status::passed(tr_fill("It answered: {}", short.trim()));
        }
        Event::Failed {
            conversation,
            message,
        } if sheet.asked.as_ref() == Some(conversation) => {
            sheet.asked = None;
            sheet.shortcut = Status::failed(message.clone());
        }
        Event::Cancelled { conversation } if sheet.asked.as_ref() == Some(conversation) => {
            sheet.asked = None;
            sheet.shortcut = Status::pending(tr("Waiting for your question"));
        }
        _ => return false,
    }
    true
}

/// A job's answer, for the sheet.
pub(super) fn receive(ui: &mut Ui, done: super::Done) {
    use super::Done;
    if let Done::Installed(Ok(())) = done {
        // Installed: read it again, sheet or not.
        if let Some(sheet) = ui.chats.setup.as_mut() {
            sheet.installing = false;
            sheet.install_error = None;
            sheet.desktop = None;
        }
        recheck(ui);
        return;
    }
    let Some(sheet) = ui.chats.setup.as_mut() else {
        return;
    };
    match done {
        Done::Detected(_) => sheet.detecting = false,
        Done::Desktop(found) => {
            sheet.desktop_checking = false;
            sheet.desktop = Some(found);
        }
        Done::Permissions(read) => {
            sheet.permissions_checking = false;
            sheet.permissions = Some(read);
        }
        Done::Installed(Err(error)) => {
            sheet.installing = false;
            sheet.install_error = Some(error);
        }
        Done::TestStarted(index) => {
            if let Some(status) = sheet.tests.get_mut(index) {
                *status = Status::checking(tr("Checking…"));
            }
        }
        Done::Test(index, result) => {
            if let Some(status) = sheet.tests.get_mut(index) {
                *status = match result {
                    Ok(saw) => Status::passed(saw),
                    Err(why) => Status::failed(why),
                };
            }
        }
        Done::Installed(Ok(())) | Done::KeySaved(_) | Done::Voices(_) => {}
    }
}

/// Reads the live checks again every few seconds while the sheet shows.
pub(super) fn poll(ui: &mut Ui, tx: &mut rune::Tx) -> bool {
    let Some(sheet) = ui.chats.setup.as_ref() else {
        return false;
    };
    if !ui.chats.live() {
        return false;
    }
    let due = sheet
        .checked_at
        .is_none_or(|at| at.elapsed() >= RECHECK);
    if due {
        recheck(ui);
    }
    tx.wake_after(RECHECK);
    false
}

/// The agents a scene finds: Claude Code and Codex ready, OpenCode not
/// running, no API key.
pub(in crate::rune_ui) fn fixture_found() -> Vec<Found> {
    BACKENDS
        .iter()
        .map(|backend| {
            let (version, ready, note) = match backend {
                AssistantBackend::ClaudeCode => (Some("2.4.1"), true, None),
                AssistantBackend::Codex => (Some("0.91.0"), true, None),
                AssistantBackend::OpenCode => {
                    (Some("1.12.3"), false, Some("Its service isn't running"))
                }
                AssistantBackend::ClaudeApi => (None, false, Some("No API key saved")),
            };
            Found {
                backend: *backend,
                path: version.map(|_| format!("/opt/homebrew/bin/{}", backend_name(*backend)).into()),
                version: version.map(str::to_owned),
                ready,
                note: note.map(str::to_owned),
            }
        })
        .collect()
}

/// The sheet for scenes: part way (`done` false), or every check passed.
pub(in crate::rune_ui) fn fixture(ui: &mut Ui, done: bool) {
    ui.chats.settings.found = Some(fixture_found());
    let mut sheet = SetupSheet::new();
    sheet.desktop = Some(AgentDesktop {
        path: Some("/opt/homebrew/bin/agent-desktop".into()),
        version: Some("0.4.2".into()),
        accessibility: true,
        screen_recording: done,
    });
    sheet.permissions = Some(Permissions {
        accessibility: true,
        screen_recording: true,
    });
    sheet.tests_ran = true;
    sheet.tests = if done {
        [
            Status::passed("Claude Code answered “OK” in 2.1 s"),
            Status::passed("agent-desktop 0.4.2"),
            Status::passed("Read 14 elements in Finder"),
            Status::passed("Claude Code listed Finder's window"),
        ]
    } else {
        [
            Status::passed("Claude Code answered “OK” in 2.1 s"),
            Status::passed("agent-desktop 0.4.2"),
            Status::failed("agent-desktop can't record the screen yet"),
            Status::checking(tr("Checking…")),
        ]
    };
    sheet.shortcut = if done {
        Status::passed("It answered: It's 9:41 PM in Tokyo.")
    } else {
        Status::pending(tr("Waiting for your question"))
    };
    ui.platform.permissions.microphone = PermissionState::Ready;
    ui.platform.permissions.accessibility = PermissionState::Ready;
    ui.chats.setup = Some(sheet);
}
