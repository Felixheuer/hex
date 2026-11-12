//! Chats (the assistant, macOS): every conversation with the assistant
//! and how it ended, the selected one's transcript with its answers
//! streaming in, a composer that asks about the window in front (Here) or
//! starts a Background task, the running tasks with Take over, Hand back,
//! and Stop, and the
//! assistant's settings. The setup sheet (`setup`) checks, live, that every
//! piece the assistant needs works on this Mac.
//!
//! The pane reads and asks `crate::assistant` only once the window connects
//! to it (`connect`); before that, and in every scene and test, it works on
//! an in-memory [`fixture::Store`] and never touches the service.

mod agent_picker;
mod agents;
mod bridge;
mod composer;
mod displays;
mod events;
pub(super) mod fixture;
mod follow;
mod jobs;
mod list;
mod live_view;
mod schedules;
mod settings;
mod setup;
mod sheet;
mod shortcut;
mod site_sign_in;
mod source;
mod strip;
mod switch;
#[cfg(all(test, target_os = "macos"))]
mod tests;
mod transcript;
mod work;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::{El, TextEdit, Tx, Waker, text};

use super::controls::{ButtonKind, Paint, button, segmented_tx};
use super::{Pane, Ui};
use crate::app_settings::AssistantBackend;
use crate::assistant::{
    Context, Conversation, ConversationId, ConversationSummary, Event, Origin, Request, TaskState,
    TaskStatus,
};
use crate::desktop::i18n::{tr, tr_fill};
use source::Source;

pub(super) use fixture::scene as fixture_scene;
pub(super) use jobs::{Done, Job, Jobs};
pub(super) use setup::Page;
pub(crate) use setup::tutorial::tasks_opened;
pub(super) use sheet::overlay as setup_overlay;

/// The sidebar's Chats icon: a speech bubble.
pub(super) static CHATS_ICON: rune::Icon = rune::Icon {
    name: "hex-chats",
    view: 24.0,
    body: include_str!("../../assets/icons/chats.svg"),
};

/// How long typing pauses before a model name is saved.
const MODEL_SAVE_DELAY: Duration = Duration::from_millis(400);

/// Which view the pane shows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum View {
    #[default]
    Chats,
    Settings,
}

/// What a request is doing between its first event and its last.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    /// Sent from the composer; the service hasn't said anything yet.
    Sending,
    /// Waiting for a free slot.
    Queued,
    Thinking,
    /// Doing something on the desktop.
    Working,
    /// The answer is streaming in.
    Answering,
}

/// A request in flight, as the transcript and the list show it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Turn {
    pub(super) question: String,
    pub(super) origin: Origin,
    pub(super) phase: Phase,
    /// Its steps so far, as a person reads them.
    pub(super) steps: Vec<String>,
    /// The answer's text so far.
    pub(super) answer: String,
    /// What the person sent while it worked, in order (steering).
    pub(super) steered: Vec<String>,
    /// Questions it asked while it worked, each with the person's answer.
    pub(super) answers: Vec<(String, String)>,
    /// When it started (Unix ms), for the plan's elapsed time.
    pub(super) started_ms: u64,
}

impl Turn {
    fn new(question: String, origin: Origin, phase: Phase) -> Self {
        Self {
            question,
            origin,
            phase,
            steps: Vec::new(),
            answer: String::new(),
            steered: Vec::new(),
            answers: Vec::new(),
            started_ms: crate::history::now_ms(),
        }
    }
}

/// Where the composer sends a new message.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SendMode {
    /// The assistant decides: about the window in front, or a job of its
    /// own (it can move a Here question to the background as it grows).
    #[default]
    Auto,
    Here,
    Background,
}

/// The most steps a turn keeps while it runs; the stored conversation has
/// them all.
const MAX_STEPS: usize = 200;

/// Why a running task isn't moving: the person has it, or its agent hit a
/// usage limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Hold {
    /// Taken over: its desktop actions wait until it's handed back.
    Paused,
    /// Retries at `until_ms` (Unix ms).
    Waiting { until_ms: u64, reason: String },
    /// It asked the person something and waits for the answer.
    Asking(String),
}

/// The Chats pane's state.
pub(super) struct ChatsPane {
    pub(super) view: View,
    source: Source,
    /// Events the service reported, until the next poll reads them.
    inbox: Arc<Mutex<Vec<Event>>>,
    /// The stored conversations, newest first; read while the pane shows.
    pub(super) list: Vec<ConversationSummary>,
    /// Read since the pane last showed, or since the store last changed.
    listed: bool,
    pub(super) selected: Option<ConversationId>,
    /// The selected conversation as stored, once read.
    pub(super) shown: Option<Conversation>,
    /// Requests in flight, by conversation.
    pub(super) turns: HashMap<ConversationId, Turn>,
    /// Why a conversation's last request failed, until it asks again.
    pub(super) failures: HashMap<ConversationId, String>,
    pub(super) tasks: Vec<TaskStatus>,
    /// Tasks taken over or waiting on a usage limit, by conversation, as
    /// their events said (the reason isn't in `TaskStatus`).
    pub(super) holds: HashMap<ConversationId, Hold>,
    /// The Here task moving the real pointer or keys now.
    pub(super) hands_off: Option<ConversationId>,
    pub(super) search: TextEdit,
    pub(super) composer: TextEdit,
    /// Where the composer sends a new message: Auto, or Here or Background.
    pub(super) lane: SendMode,
    /// Each running job's plan, by conversation.
    pub(super) plans: HashMap<ConversationId, Vec<crate::assistant::PlanStep>>,
    /// Plans folded to their one line.
    pub(super) plan_folded: Vec<ConversationId>,
    /// The question each waiting job asked.
    pub(super) questions: HashMap<ConversationId, String>,
    /// The answer to the shown conversation's question, as typed.
    pub(super) answer: TextEdit,
    /// A question arrived in the shown conversation: its field takes focus.
    pub(super) focus_answer: bool,
    /// Here conversations that became Background jobs while shown.
    pub(super) escalated: Vec<ConversationId>,
    /// The scheduled requests, as last read.
    pub(super) schedules: Vec<bridge::schedules::Schedule>,
    /// The New schedule sheet, while it shows.
    pub(super) new_schedule: Option<schedules::NewSchedule>,
    /// The files in each job folder an answer named, by folder.
    pub(super) folders: HashMap<String, Vec<String>>,
    /// A conversation whose Delete waits for its confirmation.
    pub(super) delete_armed: Option<ConversationId>,
    /// Step blocks the person unfolded, by key.
    pub(super) unfolded: Vec<String>,
    /// The conversation last named to the service as the one showing.
    focus: Option<ConversationId>,
    /// Why the last action failed, above the list.
    pub(super) error: Option<String>,
    /// Screenshot thumbnails.
    pub(super) images: transcript::Thumbnails,
    /// Detection, installs, and checks off the window's thread.
    pub(super) jobs: Jobs,
    /// The setup flow, while it shows.
    pub(super) setup: Option<setup::SetupFlow>,
    /// The assistant settings' text fields and lists.
    pub(super) settings: settings::SettingsState,
    /// Following new activity, and the transcript's scroll.
    pub(super) follow: follow::Follow,
    /// The voice first run handed over to the assistant's pages already.
    pub(super) handed_off: bool,
    /// A task's browser shown large (task, session), from its preview.
    pub(super) live: Option<(ConversationId, String)>,
}

impl Default for ChatsPane {
    fn default() -> Self {
        Self {
            view: View::Chats,
            source: Source::Fixture(Box::default()),
            inbox: Arc::default(),
            list: Vec::new(),
            listed: false,
            selected: None,
            shown: None,
            turns: HashMap::new(),
            failures: HashMap::new(),
            tasks: Vec::new(),
            holds: HashMap::new(),
            hands_off: None,
            search: TextEdit::default(),
            composer: TextEdit::default(),
            lane: SendMode::Auto,
            plans: HashMap::new(),
            plan_folded: Vec::new(),
            questions: HashMap::new(),
            answer: TextEdit::default(),
            focus_answer: false,
            escalated: Vec::new(),
            folders: HashMap::new(),
            schedules: Vec::new(),
            new_schedule: None,
            delete_armed: None,
            unfolded: Vec::new(),
            focus: None,
            error: None,
            images: transcript::Thumbnails::default(),
            jobs: Jobs::default(),
            setup: None,
            settings: settings::SettingsState::default(),
            follow: follow::Follow::default(),
            handed_off: false,
            live: None,
        }
    }
}

impl ChatsPane {
    /// The pane over `store`, for scenes and tests.
    pub(super) fn fixture(store: fixture::Store) -> Self {
        let mut pane = Self {
            source: Source::Fixture(Box::new(store)),
            ..Self::default()
        };
        pane.reload();
        pane
    }

    /// The fixture store, in scenes and tests.
    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn store(&self) -> Option<&fixture::Store> {
        match &self.source {
            Source::Fixture(store) => Some(store.as_ref()),
            Source::Live => None,
        }
    }

    fn live(&self) -> bool {
        matches!(self.source, Source::Live)
    }

    /// An event as the service reports it, for the next poll (tests).
    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn deliver(&self, event: Event) {
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(event);
        }
    }

    /// Reads the list, the shown conversation, and the tasks again.
    fn reload(&mut self) {
        self.list = self.source.conversations();
        self.listed = true;
        self.reload_shown();
        self.tasks = self.source.tasks();
        self.schedules = self.source.schedules();
    }

    fn reload_shown(&mut self) {
        self.shown = self
            .selected
            .as_deref()
            .and_then(|id| self.source.conversation(id));
        // The job folder its answer names, listed once.
        if let Some(folder) = self.shown.as_ref().and_then(work::folder_of) {
            let files = self.source.files(&folder);
            self.folders.insert(folder, files);
        }
        if let Some(id) = self.selected.clone()
            && self.live()
            && !self.plans.contains_key(&id)
        {
            let plan = crate::assistant::service::plan(&id);
            if !plan.is_empty() {
                self.plans.insert(id.clone(), plan);
            }
            if let Some(question) = crate::assistant::service::question(&id) {
                self.questions.insert(id, question);
            }
        }
    }

    /// Shows conversation `id`.
    pub(super) fn select(&mut self, id: ConversationId) {
        if self.selected.as_deref() == Some(id.as_str()) && self.shown.is_some() {
            return;
        }
        self.delete_armed = None;
        self.selected = Some(id);
        self.reload_shown();
    }

    /// Starts a new conversation: nothing selected, the composer empty.
    /// Chats stops following until something is picked again.
    pub(super) fn new_chat(&mut self) {
        self.follow.pinned = true;
        self.selected = None;
        self.shown = None;
        self.delete_armed = None;
    }

    /// Whether `id` has a request in flight.
    pub(super) fn running(&self, id: &str) -> bool {
        self.turns.contains_key(id) || self.tasks.iter().any(|task| task.conversation == id)
    }

    /// Sends the composer's text into the shown conversation (a new one
    /// when none shows). While that conversation runs, it steers it: the
    /// agent hears it as the person interrupting. Nothing for an empty
    /// composer.
    pub(super) fn send(&mut self) -> bool {
        let question = self.composer.text.trim().to_owned();
        if question.is_empty() {
            return false;
        }
        if let Some(id) = self.selected.clone()
            && self.running(&id)
        {
            if self.questions.contains_key(&id) {
                self.composer.clear();
                return self.answer_question(&id, question);
            }
            return self.steer(id, question);
        }
        // Here asks about the window in front of Hex, read now; Background
        // looks for itself; Auto lets the assistant choose, knowing the
        // window in front.
        let (origin, context) = match self.lane {
            SendMode::Here => (Origin::ActionShortcut, self.source.here_context()),
            SendMode::Auto => (Origin::Chat, self.source.here_context()),
            SendMode::Background => (Origin::Chat, Context::default()),
        };
        let id = self.source.submit(Request {
            conversation: self.selected.clone(),
            text: question.clone(),
            origin,
            context,
        });
        if id.is_empty() {
            self.error =
                Some(tr("The assistant didn't take the message. Try again in a moment.").into());
            return true;
        }
        self.error = None;
        self.composer.clear();
        self.failures.remove(&id);
        self.turns
            .insert(id.clone(), Turn::new(question, origin, Phase::Sending));
        self.delete_armed = None;
        self.follow.to_end = true;
        if self.selected.as_deref() != Some(id.as_str()) {
            self.selected = Some(id);
            self.reload_shown();
        }
        true
    }

    /// Sends `words` to running conversation `id`, shown in its turn as
    /// sent while it worked.
    fn steer(&mut self, id: ConversationId, words: String) -> bool {
        let origin = self
            .turns
            .get(&id)
            .map(|turn| turn.origin)
            .or_else(|| self.task(&id).map(|task| task.origin))
            .unwrap_or(Origin::Chat);
        let sent = self.source.submit(Request {
            conversation: Some(id.clone()),
            text: words.clone(),
            origin,
            context: Context::default(),
        });
        if sent.is_empty() {
            self.error =
                Some(tr("The assistant didn't take the message. Try again in a moment.").into());
            return true;
        }
        self.error = None;
        self.composer.clear();
        let question = self
            .task(&id)
            .map(|task| task.title.clone())
            .unwrap_or_default();
        self.turns
            .entry(id)
            .or_insert_with(|| Turn::new(question, origin, Phase::Working))
            .steered
            .push(words);
        self.follow.to_end = true;
        true
    }

    pub(super) fn stop(&mut self, id: &str) {
        self.source.cancel(id);
    }

    pub(super) fn stop_all(&mut self) {
        let ids: Vec<_> = self
            .tasks
            .iter()
            .map(|task| task.conversation.clone())
            .collect();
        self.source.stop_all(ids);
    }

    /// The person takes task `id` over: it waits while they work.
    pub(super) fn take_over(&mut self, id: &str) {
        self.source.take_over(id, true);
        self.holds.insert(id.to_owned(), Hold::Paused);
        self.tasks = self.source.tasks();
    }

    /// Gives task `id` back; it looks again before acting.
    pub(super) fn hand_back(&mut self, id: &str) {
        self.source.take_over(id, false);
        self.holds.remove(id);
        self.tasks = self.source.tasks();
    }

    /// Continues conversation `id` after Hex quit while it ran.
    pub(super) fn resume(&mut self, id: &str) {
        let Some(resumed) = self.source.resume(id) else {
            self.error = Some(tr("Couldn't resume this task. Try asking again.").into());
            return;
        };
        self.error = None;
        let question = self
            .shown
            .as_ref()
            .filter(|shown| shown.id == id)
            .and_then(|shown| {
                shown
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.role == crate::assistant::Role::User)
            })
            .map(|message| message.text().trim().to_owned())
            .unwrap_or_default();
        let origin = self
            .list
            .iter()
            .find(|row| row.id == id)
            .map_or(Origin::Background, |row| row.origin);
        self.failures.remove(&resumed);
        self.turns
            .insert(resumed.clone(), Turn::new(question, origin, Phase::Sending));
        self.follow.to_end = true;
        self.select(resumed);
    }

    /// Why `task` isn't moving, if it isn't: what its events said, else its
    /// state as the service lists it.
    pub(super) fn hold(&self, task: &TaskStatus) -> Option<Hold> {
        if let Some(hold) = self.holds.get(&task.conversation) {
            return Some(hold.clone());
        }
        if let Some(question) = self.questions.get(&task.conversation) {
            return Some(Hold::Asking(question.clone()));
        }
        match task.state {
            TaskState::NeedsInput => Some(Hold::Asking(String::new())),
            TaskState::Paused => Some(Hold::Paused),
            TaskState::Waiting { until_ms } => Some(Hold::Waiting {
                until_ms,
                reason: String::new(),
            }),
            TaskState::Queued | TaskState::Running => None,
        }
    }

    /// The task running conversation `id`, if one is.
    pub(super) fn task(&self, id: &str) -> Option<&TaskStatus> {
        self.tasks.iter().find(|task| task.conversation == id)
    }

    /// Deletes the armed conversation.
    pub(super) fn delete_armed(&mut self) {
        let Some(id) = self.delete_armed.take() else {
            return;
        };
        self.source.delete(&id);
        self.turns.remove(&id);
        self.failures.remove(&id);
        if self.selected.as_ref() == Some(&id) {
            self.new_chat();
        }
        self.list = self.source.conversations();
    }

    /// The rows the list shows: stored conversations, and new ones whose
    /// first request is still in flight, matching the search.
    pub(super) fn rows(&self) -> Vec<ConversationSummary> {
        let mut rows: Vec<ConversationSummary> = self
            .turns
            .iter()
            .filter(|(id, _)| !self.list.iter().any(|row| &row.id == *id))
            .map(|(id, turn)| ConversationSummary {
                id: id.clone(),
                title: turn.question.clone(),
                updated_ms: crate::history::now_ms(),
                origin: turn.origin,
                preview: String::new(),
                outcome: crate::assistant::Outcome::Running,
            })
            .collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.extend(self.list.iter().cloned());
        let query = self.search.text.trim().to_lowercase();
        if query.is_empty() {
            return rows;
        }
        rows.into_iter()
            .filter(|row| {
                row.title.to_lowercase().contains(&query)
                    || row.preview.to_lowercase().contains(&query)
            })
            .collect()
    }
}

/// Whether the pane is in the sidebar: on every desktop, whether or not
/// the assistant is on (off, it offers to turn it on).
pub(super) fn available(ui: &Ui) -> bool {
    let _ = ui;
    crate::assistant::available()
}

/// Whether the assistant runs: the pane is here and its switch is on.
fn on(ui: &Ui) -> bool {
    available(ui) && crate::assistant::enabled(&ui.settings)
}

/// Connects the pane to the assistant service: the window shows what it
/// does from now on, woken for every event.
pub(super) fn connect(ui: &mut Ui, waker: Waker) {
    let pane = &mut ui.chats;
    pane.source = Source::Live;
    pane.listed = false;
    pane.jobs.connect(waker.clone());
    pane.images.connect(waker.clone());
    let inbox = pane.inbox.clone();
    let woken = waker.clone();
    crate::assistant::subscribe(Box::new(move |event: &Event| {
        if let Ok(mut events) = inbox.lock() {
            events.push(event.clone());
        }
        waker();
    }));
    // A browser's preview pressed in the orb's list, or its page changing.
    super::assistant_overlay::live::on_change(move || woken());
    // A relaunch reopens the onboarding where it was.
    setup::resume::launch(ui);
}

/// Opens the window's Chats pane on `conversation`, from the menu bar, a
/// `hex://chats?id=` link, the HUD's bubble, or its task stack; without
/// one, on the most relevant conversation.
pub(super) fn show(ui: &mut Ui, conversation: Option<ConversationId>, tx: &mut Tx) {
    if !on(ui) {
        return;
    }
    ui.select_pane(Pane::Chats, tx);
    ui.chats.view = View::Chats;
    ui.chats.reload();
    match conversation {
        Some(id) => ui.chats.land(id),
        None => ui.chats.auto_select(),
    }
    ui.chats.follow.landed = true;
}

/// Opens the assistant's onboarding, and starts its checks.
pub(super) fn open_setup(ui: &mut Ui) {
    setup::open(ui);
}

/// A hold of the Action shortcut before setup finished: the onboarding
/// where it was left (or as it is, when it shows), saying so.
pub(super) fn open_setup_from_hold(ui: &mut Ui, tx: &mut Tx) {
    if !on(ui) || ui.settings.assistant.setup_done {
        return;
    }
    if ui.pane != Pane::Chats {
        ui.select_pane(Pane::Chats, tx);
    }
    if ui.chats.setup.is_none() {
        setup::open(ui);
    }
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.from_hold = true;
    }
}

/// Runs before every frame: takes in the service's events, reads the list
/// while the pane shows, names the conversation showing to the service,
/// and follows the setup sheet and settings. True when anything shown
/// changed.
pub(super) fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    let mut changed = false;
    let events = ui
        .chats
        .inbox
        .lock()
        .map(|mut inbox| std::mem::take(&mut *inbox))
        .unwrap_or_default();
    for event in &events {
        changed |= ui.chats.apply(event);
        changed |= setup::event(ui, event);
    }
    changed |= jobs::poll(ui);
    if !on(ui) {
        if ui.chats.setup.is_some() {
            // The assistant turned off: its onboarding closes; the pane
            // stays, offering to turn it back on.
            setup::close(ui);
            changed = true;
        }
        if !available(ui) && ui.pane == Pane::Chats {
            ui.select_pane(Pane::General, tx);
            changed = true;
        }
        set_focus(ui, None);
        changed |= settings::poll(ui, Instant::now());
        return changed;
    }
    changed |= setup::resume::follow_first_run(ui);
    if let Some((task, session)) = super::assistant_overlay::live::take_request() {
        live_view::open(ui, task, session);
        changed = true;
    }
    let visible = ui.window.as_ref().is_none_or(rune::AppHandle::is_visible);
    let showing = visible && ui.pane == Pane::Chats && !ui.labs_open;
    if showing && !ui.chats.listed {
        ui.chats.reload();
        changed = true;
    }
    if showing && !ui.chats.follow.was_showing && !ui.chats.follow.landed {
        // Brought forward: the most relevant conversation.
        ui.chats.auto_select();
        changed = true;
    }
    ui.chats.follow.was_showing = showing;
    ui.chats.follow.landed = false;
    if !showing {
        // Read again when it next shows.
        ui.chats.listed = false;
        if ui.chats.live.is_some() {
            // Nothing shows it: its stream stops.
            live_view::close(ui);
            changed = true;
        }
    }
    let now = Instant::now();
    ui.chats.watch(now);
    if std::mem::take(&mut ui.chats.follow.to_end) {
        tx.scroll_to_bottom_instant(follow::SCROLL);
    }
    if ui
        .chats
        .holds
        .values()
        .any(|hold| matches!(hold, Hold::Waiting { .. }))
        || ui
            .chats
            .tasks
            .iter()
            .any(|task| matches!(task.state, TaskState::Waiting { .. }))
    {
        // The countdown ticks.
        tx.wake_after(Duration::from_secs(1));
    }
    let focus = if showing && ui.chats.view == View::Chats {
        ui.chats.selected.clone()
    } else {
        None
    };
    changed |= set_focus(ui, focus);
    work::poll(ui, tx);
    changed |= setup::poll(ui, tx);
    changed |= settings::poll(ui, now);
    changed
}

/// Tells the service which conversation shows, when that changed.
fn set_focus(ui: &mut Ui, focus: Option<ConversationId>) -> bool {
    if ui.chats.focus == focus {
        return false;
    }
    ui.chats.focus = focus.clone();
    ui.chats.source.set_focus(focus);
    false
}

/// The window closed: no conversation shows any more.
pub(super) fn window_closed(ui: &mut Ui) {
    set_focus(ui, None);
    ui.chats.delete_armed = None;
}

/// Escape: closes a browser shown large, or disarms a delete. True when it
/// did.
pub(super) fn escape(ui: &mut Ui) -> bool {
    if ui.chats.live.is_some() {
        return live_view::close(ui);
    }
    ui.chats.delete_armed.take().is_some()
}

/// The pane: the setup notice, the running tasks, then the conversations
/// and the transcript, or the settings.
pub(super) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> (Vec<El>, El) {
    match ui.chats.view {
        View::Settings => (
            // Off, there is nothing to set up until it's turned on.
            crate::assistant::enabled(&ui.settings)
                .then(|| setup_button(p))
                .into_iter()
                .collect(),
            super::scroll_body(p.k, "chats-settings-scroll", settings::page(ui, p, layer)),
        ),
        View::Chats if !crate::assistant::enabled(&ui.settings) => {
            (Vec::new(), switch::off_card(ui, p))
        }
        View::Chats => {
            layer.extend(schedules::sheet(ui, p));
            layer.extend(live_view::sheet(ui, p));
            (header_actions(ui, p), list::body(ui, p, layer))
        }
    }
}

/// The header's `Chats | Settings` switch, beside the title.
pub(super) fn view_switch(ui: &Ui, p: Paint) -> El {
    let views = [tr("Conversations"), tr("Settings")];
    let selected = match ui.chats.view {
        View::Chats => 0,
        View::Settings => 1,
    };
    segmented_tx(
        p,
        "chats-view",
        &views,
        selected,
        |ui: &mut Ui, index: usize, tx: &mut Tx| {
            let view = if index == 0 {
                View::Chats
            } else {
                View::Settings
            };
            if ui.chats.view != view {
                tx.settle();
                settings::flush(ui);
                ui.open = None;
                ui.chats.view = view;
                ui.chats.delete_armed = None;
                if view == View::Settings {
                    settings::opened(ui);
                }
            }
        },
    )
}

/// The agent's name, as Settings and the header show it.
pub(super) fn backend_name(backend: AssistantBackend) -> &'static str {
    match backend {
        AssistantBackend::ClaudeCode => "Claude Code",
        AssistantBackend::Codex => "Codex",
        AssistantBackend::OpenCode => "OpenCode",
        AssistantBackend::ClaudeApi => "Claude API",
    }
}

/// Every backend, in the order Settings and setup list them.
pub(super) const BACKENDS: [AssistantBackend; 4] = [
    AssistantBackend::ClaudeCode,
    AssistantBackend::Codex,
    AssistantBackend::OpenCode,
    AssistantBackend::ClaudeApi,
];

/// The Chats view's header: which agent answers, how many tasks run with
/// Stop all while any does, and New chat.
fn header_actions(ui: &Ui, p: Paint) -> Vec<El> {
    let backend = text(backend_name(ui.settings.assistant.backend))
        .key("chats-backend")
        .font(p.k.ty.small)
        .color(p.faint())
        .help(tr("The agent that answers. Change it in Settings"));
    let new_chat = button(
        p,
        "new-chat",
        tr("New chat"),
        ButtonKind::Secondary,
        |ui: &mut Ui, tx: &mut Tx| {
            ui.chats.new_chat();
            tx.focus("composer-input", false);
        },
    );
    let mut actions = vec![backend];
    actions.extend(strip::header_running(ui, p));
    actions.push(new_chat);
    actions
}

/// Settings' header action: the setup sheet again.
fn setup_button(p: Paint) -> El {
    button(
        p,
        "run-assistant-setup",
        tr("Run setup…"),
        ButtonKind::Secondary,
        |ui: &mut Ui, _| open_setup(ui),
    )
}

/// A compact age for the list: Now, 5m ago, 3h ago, 2d ago.
pub(super) fn age(at_ms: u64, now_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(at_ms) / 1_000;
    let (template, value) = match seconds {
        0..=59 => return tr("Now").into(),
        60..=3_599 => ("{}m ago", seconds / 60),
        3_600..=86_399 => ("{}h ago", seconds / 3_600),
        _ => ("{}d ago", seconds / 86_400),
    };
    tr_fill(template, &value.to_string())
}
