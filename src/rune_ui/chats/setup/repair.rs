//! "Hex needs your help for a moment": after setup was finished, the
//! onboarding comes back for what's missing now, and only for that
//! (`desktop::setup_gaps`). Hex looks at launch and every few minutes, off
//! the window's thread (`Job::Gaps`); when something is missing it opens the
//! flow at the first page that fixes it, shows just the pages needed, says
//! what changed at the top, moves on by itself once a page works, and
//! closes after the last. Later puts it off until Hex next starts. What no
//! page fixes is done with one action (the Hex voice downloads again).
//!
//! A hold of the Action shortcut while something essential is missing says
//! what (`crate::assistant::setup_gap`) and opens the guide, so it's found
//! without the window too.

use std::time::{Duration, Instant};

use super::super::Ui;
use super::{Page, SetupFlow, go, open_at, page_done};
use crate::app_settings::{AppSettings, HotkeySlot};
use crate::desktop::setup_gaps::{self, Facts, Fix, Gap, SETUP_VERSION};

/// How often Hex looks again while nothing shows.
pub(in crate::rune_ui) const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long a page that works stays before the guide moves on, so the
/// person sees it turn green.
const SETTLE: Duration = Duration::from_millis(900);

/// What the launch check found, and whether the person put it off.
#[derive(Debug, Default)]
pub(in crate::rune_ui) struct Watch {
    pub(in crate::rune_ui) gaps: Vec<Gap>,
    checked_at: Option<Instant>,
    checking: bool,
    /// Later was pressed: the guide stays closed until Hex next starts.
    pub(in crate::rune_ui) snoozed: bool,
}

/// The guide inside the setup flow: the pages it shows, in order, and why.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::rune_ui) struct Repair {
    pub(in crate::rune_ui) pages: Vec<Page>,
    pub(in crate::rune_ui) gaps: Vec<Gap>,
    /// When the page showing started to work.
    done_at: Option<Instant>,
}

impl Repair {
    pub(in crate::rune_ui) fn new(gaps: Vec<Gap>) -> Self {
        Self {
            pages: pages_for(&gaps),
            gaps,
            done_at: None,
        }
    }

    /// The page after `page` in the guide; `None` after the last.
    pub(in crate::rune_ui) fn next(&self, page: Page) -> Option<Page> {
        let at = self.pages.iter().position(|shown| *shown == page)?;
        self.pages.get(at + 1).copied()
    }

    pub(in crate::rune_ui) fn previous(&self, page: Page) -> Option<Page> {
        let at = self.pages.iter().position(|shown| *shown == page)?;
        at.checked_sub(1).map(|at| self.pages[at])
    }
}

/// The pages that fix `gaps`, in the flow's order.
pub(in crate::rune_ui) fn pages_for(gaps: &[Gap]) -> Vec<Page> {
    let order: Vec<&'static str> = Page::shown().iter().map(|page| page.name()).collect();
    setup_gaps::pages_for(gaps, &order)
        .into_iter()
        .filter_map(Page::named)
        .collect()
}

/// Reads this Mac for [`setup_gaps::setup_gaps`]. Blocks for a moment
/// (agent detection, agent-desktop's own report); off the window's thread.
pub(in crate::rune_ui) fn read_facts(settings: &AppSettings) -> Facts {
    use crate::assistant::backends;
    use crate::desktop::onboarding::{self, PermissionState};
    let assistant = &settings.assistant;
    let status = onboarding::status(&settings.transcription);
    let permissions = crate::assistant::computer::permissions_now();
    let found = backends::detect();
    let agent = found
        .iter()
        .find(|found| found.backend == assistant.backend);
    let desktop = if assistant.control_desktop {
        Some(backends::agent_desktop())
    } else {
        None
    };
    let browser = assistant
        .background_browser
        .then(crate::assistant::browser::agent_browser);
    let shortcut_taken = super::super::shortcut::conflicts(HotkeySlot::Action, settings)
        .iter()
        .any(|conflict| conflict.taken);
    let hex_voice_selected = assistant
        .voice
        .as_deref()
        .is_some_and(|voice| crate::assistant::speech::neural::preset(voice).is_some());
    Facts {
        copy_problem: crate::desktop::permission_doctor::diagnose().is_some(),
        microphone: status.microphone == PermissionState::Ready,
        accessibility: status.accessibility == PermissionState::Ready,
        screen_recording: permissions.screen_recording,
        automation: {
            use crate::desktop::automation::{self, AutomationState};
            !matches!(
                automation::refresh(),
                AutomationState::NotAsked | AutomationState::Denied
            )
        },
        personal_data: crate::desktop::data_access::all_answered(crate::desktop::data_access::read),
        transcription_model: status.transcription_model,
        agent_installed: agent.is_some_and(|agent| {
            agent.path.is_some()
                || agent.backend == crate::app_settings::AssistantBackend::ClaudeApi
        }),
        agent_ready: agent.is_some_and(|agent| agent.ready),
        desktop_wanted: assistant.control_desktop,
        agent_desktop_installed: desktop.as_ref().is_some_and(|found| found.path.is_some()),
        agent_desktop_grants: desktop
            .as_ref()
            .is_some_and(|found| found.accessibility && found.screen_recording),
        browser_wanted: assistant.background_browser,
        agent_browser_installed: browser.as_ref().is_some_and(|found| found.path.is_some()),
        shortcut_taken,
        hex_voice_selected,
        hex_voice_installed: crate::assistant::speech::neural::files::installed(),
        setup_version: assistant.setup_version,
    }
}

/// Starts a check when one is due: at launch, then every [`CHECK_EVERY`],
/// once setup was finished. Live only.
pub(in crate::rune_ui) fn poll(ui: &mut Ui, tx: &mut rune::Tx) -> bool {
    let changed = advance_when_fixed(ui);
    if ui
        .chats
        .setup
        .as_ref()
        .and_then(|flow| flow.repair.as_ref())
        .is_some_and(|repair| repair.done_at.is_some())
    {
        tx.wake_after(SETTLE);
    }
    if !ui.chats.live()
        || !crate::assistant::enabled(&ui.settings)
        || !ui.settings.assistant.setup_done
    {
        return changed;
    }
    let watch = &mut ui.chats.repair;
    let due = watch
        .checked_at
        .is_none_or(|at| at.elapsed() >= CHECK_EVERY);
    if due && !watch.checking {
        watch.checking = true;
        watch.checked_at = Some(Instant::now());
        let settings = ui.settings.clone();
        ui.chats
            .jobs
            .run(super::super::jobs::Job::Gaps(Box::new(settings)));
    }
    tx.wake_after(CHECK_EVERY);
    changed
}

/// A check's answer: kept, told to the shortcut, and the guide opened when
/// something is missing and nothing else shows.
pub(in crate::rune_ui) fn received(ui: &mut Ui, gaps: Vec<Gap>) {
    ui.chats.repair.checking = false;
    if ui.chats.live() {
        crate::assistant::set_setup_gap(setup_gaps::essential(&gaps).map(Gap::message));
    }
    ui.chats.repair.gaps = gaps;
    if ui.chats.repair.snoozed || ui.chats.setup.is_some() || ui.first_run.is_some() {
        return;
    }
    open(ui);
}

/// Opens the guide on what the last check found; false when no page
/// fixes any of it (an action may still have started).
pub(in crate::rune_ui) fn open(ui: &mut Ui) -> bool {
    let gaps = ui.chats.repair.gaps.clone();
    if gaps.is_empty() {
        return false;
    }
    if gaps.contains(&Gap::HexVoice) && ui.chats.settings.voice_download.is_none() {
        // The person chose that voice: it downloads again.
        ui.chats.jobs.run(super::super::jobs::Job::DownloadVoice);
    }
    let repair = Repair::new(gaps);
    let Some(first) = repair.pages.first().copied() else {
        return false;
    };
    open_at(ui, first);
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.repair = Some(repair);
    }
    true
}

/// A hold while something essential is missing (or Continue setup): the
/// guide, or for a missing speech model Settings' models.
pub(in crate::rune_ui) fn open_from_hold(ui: &mut Ui, tx: &mut rune::Tx) {
    ui.chats.repair.snoozed = false;
    if ui.chats.setup.is_some() {
        return;
    }
    if !open(ui)
        && ui
            .chats
            .repair
            .gaps
            .iter()
            .any(|gap| gap.fix() == Fix::Models)
    {
        ui.select_pane(super::super::super::Pane::Dictation, tx);
    }
}

/// Later, in the guide: closed until Hex next starts.
pub(in crate::rune_ui) fn snooze(ui: &mut Ui) {
    ui.chats.repair.snoozed = true;
    super::close(ui);
}

/// The guide's last page works: setup is whole again, at this version.
pub(in crate::rune_ui) fn finish(ui: &mut Ui) {
    if ui.settings.assistant.setup_version < SETUP_VERSION {
        ui.update(|settings| settings.assistant.setup_version = SETUP_VERSION);
    }
    // Look again soon, to clear what was fixed.
    ui.chats.repair.checked_at = None;
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.repair = None;
    }
    super::close(ui);
}

/// Moves on once the page showing has worked for [`SETTLE`]. True when the
/// page changed.
fn advance_when_fixed(ui: &mut Ui) -> bool {
    let Some(flow) = ui.chats.setup.as_ref() else {
        return false;
    };
    let Some(repair) = flow.repair.as_ref() else {
        return false;
    };
    let page = flow.page;
    let works = page_done(ui, flow, page);
    let now = Instant::now();
    let move_on = match (works, repair.done_at) {
        (false, _) => {
            set_done_at(ui, None);
            return false;
        }
        (true, None) => {
            set_done_at(ui, Some(now));
            return false;
        }
        (true, Some(at)) => now.duration_since(at) >= SETTLE,
    };
    if !move_on {
        return false;
    }
    let next = repair.next(page);
    set_done_at(ui, None);
    match next {
        Some(next) => go(ui, next),
        None => finish(ui),
    }
    true
}

fn set_done_at(ui: &mut Ui, at: Option<Instant>) {
    if let Some(repair) = ui
        .chats
        .setup
        .as_mut()
        .and_then(|flow| flow.repair.as_mut())
    {
        repair.done_at = at;
    }
}

/// The page after `page`: the guide's when it shows, else the flow's.
pub(in crate::rune_ui) fn next_page(flow: &SetupFlow, page: Page) -> Option<Page> {
    match &flow.repair {
        Some(repair) => repair.next(page),
        None => page.next_shown(),
    }
}

pub(in crate::rune_ui) fn previous_page(flow: &SetupFlow, page: Page) -> Option<Page> {
    match &flow.repair {
        Some(repair) => repair.previous(page),
        None => page.previous_shown(),
    }
}

/// The pages the progress marks count.
pub(in crate::rune_ui) fn pages(flow: &SetupFlow) -> Vec<Page> {
    match &flow.repair {
        Some(repair) => repair.pages.clone(),
        None => Page::shown().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_starts_at_the_first_page_that_fixes_a_gap() {
        let repair = Repair::new(vec![Gap::Shortcut, Gap::AgentSignIn, Gap::ScreenRecording]);
        // The shortcut's own page is macOS's; elsewhere its gap has none.
        if cfg!(target_os = "macos") {
            assert_eq!(repair.pages, [Page::Agent, Page::Desktop, Page::Shortcut]);
            assert_eq!(repair.next(Page::Shortcut), None);
        } else {
            assert_eq!(repair.pages, [Page::Agent, Page::Desktop]);
            assert_eq!(repair.next(Page::Desktop), None);
        }
        assert_eq!(repair.next(Page::Agent), Some(Page::Desktop));
        assert_eq!(repair.previous(Page::Desktop), Some(Page::Agent));
        assert_eq!(repair.previous(Page::Agent), None);
    }

    #[test]
    fn what_no_page_fixes_opens_no_page() {
        assert!(Repair::new(vec![Gap::HexVoice]).pages.is_empty());
    }
}
