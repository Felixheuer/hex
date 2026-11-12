//! Chats (Labs › Assistant, macOS): every conversation with the assistant,
//! the selected one's transcript with its answers streaming in, a composer
//! that asks by typing, the running tasks with Stop, Agent mode, and the
//! assistant's settings. The setup sheet (`setup`) checks, live, that every
//! piece the assistant needs works on this Mac.
//!
//! The pane reads and asks `crate::assistant` only once the window connects
//! to it (`connect`); before that, and in every scene and test, it works on
//! an in-memory [`fixture::Store`] and never touches the service.

pub(super) mod fixture;
mod jobs;
mod list;
mod settings;
mod setup;
mod sheet;
mod shortcut;
#[cfg(all(test, target_os = "macos"))]
mod tests;
mod transcript;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::{El, TextEdit, Tx, Waker, row, text};

use super::controls::{ButtonKind, Paint, button, segmented_tx};
use super::{Pane, Ui};
use crate::app_settings::AssistantBackend;
use crate::assistant::{
    Context, Conversation, ConversationId, ConversationSummary, Event, Origin, Request, TaskStatus,
};
use crate::desktop::i18n::{tr, tr_fill};

pub(super) use fixture::scene as fixture_scene;
pub(super) use jobs::{Done, Job, Jobs};
pub(super) use setup::Page;
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
}

/// The most steps a turn keeps while it runs; the stored conversation has
/// them all.
const MAX_STEPS: usize = 200;

/// Where the pane reads conversations and sends requests.
enum Source {
    /// Scenes, tests, and a window that hasn't connected yet.
    Fixture(fixture::Store),
    /// The assistant service.
    Live,
}

impl Source {
    fn conversations(&self) -> Vec<ConversationSummary> {
        match self {
            Self::Fixture(store) => store.conversations(),
            Self::Live => crate::assistant::conversations(),
        }
    }

    fn conversation(&self, id: &str) -> Option<Conversation> {
        match self {
            Self::Fixture(store) => store.conversation(id),
            Self::Live => crate::assistant::conversation(id),
        }
    }

    fn tasks(&self) -> Vec<TaskStatus> {
        match self {
            Self::Fixture(store) => store.tasks.clone(),
            Self::Live => crate::assistant::tasks(),
        }
    }

    fn agent_mode(&self) -> bool {
        match self {
            Self::Fixture(store) => store.agent_mode,
            Self::Live => crate::assistant::agent_mode(),
        }
    }

    fn submit(&mut self, request: Request) -> ConversationId {
        match self {
            Self::Fixture(store) => store.submit(request),
            Self::Live => crate::assistant::submit(request),
        }
    }

    fn cancel(&mut self, id: &str) {
        match self {
            Self::Fixture(store) => store.cancelled.push(id.to_owned()),
            Self::Live => crate::assistant::cancel(id),
        }
    }

    fn delete(&mut self, id: &str) {
        match self {
            Self::Fixture(store) => store.delete(id),
            Self::Live => crate::assistant::delete(id),
        }
    }

    fn set_agent_mode(&mut self, on: bool) {
        match self {
            Self::Fixture(store) => store.agent_mode = on,
            Self::Live => crate::assistant::set_agent_mode(on),
        }
    }

    fn set_focus(&mut self, id: Option<ConversationId>) {
        match self {
            Self::Fixture(store) => store.focus = id,
            Self::Live => crate::assistant::set_focus(id),
        }
    }
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
    pub(super) agent_mode: bool,
    pub(super) search: TextEdit,
    pub(super) composer: TextEdit,
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
}

impl Default for ChatsPane {
    fn default() -> Self {
        Self {
            view: View::Chats,
            source: Source::Fixture(fixture::Store::default()),
            inbox: Arc::default(),
            list: Vec::new(),
            listed: false,
            selected: None,
            shown: None,
            turns: HashMap::new(),
            failures: HashMap::new(),
            tasks: Vec::new(),
            agent_mode: false,
            search: TextEdit::default(),
            composer: TextEdit::default(),
            delete_armed: None,
            unfolded: Vec::new(),
            focus: None,
            error: None,
            images: transcript::Thumbnails::default(),
            jobs: Jobs::default(),
            setup: None,
            settings: settings::SettingsState::default(),
        }
    }
}

impl ChatsPane {
    /// The pane over `store`, for scenes and tests.
    pub(super) fn fixture(store: fixture::Store) -> Self {
        let mut pane = Self {
            source: Source::Fixture(store),
            ..Self::default()
        };
        pane.reload();
        pane
    }

    /// The fixture store, in scenes and tests.
    #[cfg(test)]
    pub(super) fn store(&self) -> Option<&fixture::Store> {
        match &self.source {
            Source::Fixture(store) => Some(store),
            Source::Live => None,
        }
    }

    fn live(&self) -> bool {
        matches!(self.source, Source::Live)
    }

    /// An event as the service reports it, for the next poll (tests).
    #[cfg(test)]
    pub(super) fn deliver(&self, event: Event) {
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(event);
        }
    }

    /// Reads the list, the shown conversation, the tasks, and Agent mode
    /// again.
    fn reload(&mut self) {
        self.list = self.source.conversations();
        self.listed = true;
        self.reload_shown();
        self.tasks = self.source.tasks();
        self.agent_mode = self.source.agent_mode();
    }

    fn reload_shown(&mut self) {
        self.shown = self
            .selected
            .as_deref()
            .and_then(|id| self.source.conversation(id));
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
    pub(super) fn new_chat(&mut self) {
        self.selected = None;
        self.shown = None;
        self.delete_armed = None;
    }

    /// Whether `id` has a request in flight.
    pub(super) fn running(&self, id: &str) -> bool {
        self.turns.contains_key(id) || self.tasks.iter().any(|task| task.conversation == id)
    }

    /// Takes in one event. True when what the pane shows changed.
    pub(super) fn apply(&mut self, event: &Event) -> bool {
        let mut reload_list = false;
        let mut reload_shown = false;
        match event {
            Event::Queued {
                conversation,
                origin,
                question,
            }
            | Event::Started {
                conversation,
                origin,
                question,
            } => {
                let phase = if matches!(event, Event::Queued { .. }) {
                    Phase::Queued
                } else {
                    Phase::Thinking
                };
                let turn = self.turns.entry(conversation.clone()).or_insert(Turn {
                    question: question.clone(),
                    origin: *origin,
                    phase,
                    steps: Vec::new(),
                    answer: String::new(),
                });
                turn.phase = phase;
                self.failures.remove(conversation);
                reload_list = !self.list.iter().any(|row| &row.id == conversation);
            }
            Event::Thinking { conversation } => {
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.phase = Phase::Thinking;
                }
            }
            Event::Step {
                conversation,
                description,
            } => {
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.phase = Phase::Working;
                    if turn.steps.len() < MAX_STEPS {
                        turn.steps.push(description.clone());
                    }
                }
            }
            Event::Window {
                conversation,
                window,
            } => {
                if let Some(turn) = self.turns.get_mut(conversation)
                    && turn.steps.len() < MAX_STEPS
                {
                    turn.steps.push(tr_fill("Working in {}", &window.app));
                }
            }
            Event::AnswerDelta { conversation, text } => {
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.phase = Phase::Answering;
                    turn.answer.push_str(text);
                }
            }
            Event::Answer { conversation, .. } | Event::Cancelled { conversation } => {
                self.turns.remove(conversation);
                reload_list = true;
                reload_shown = self.selected.as_ref() == Some(conversation);
            }
            Event::Failed {
                conversation,
                message,
            } => {
                self.turns.remove(conversation);
                self.failures.insert(conversation.clone(), message.clone());
                reload_list = true;
                reload_shown = self.selected.as_ref() == Some(conversation);
            }
            Event::ConversationsChanged => {
                reload_list = true;
                reload_shown = true;
            }
            Event::ModeChanged { on } => self.agent_mode = *on,
            // The UI engineer shows these (paused, waiting).
            Event::Paused { .. }
            | Event::Resumed { .. }
            | Event::Waiting { .. }
            | Event::HandsOff { .. }
            | Event::Plan { .. }
            | Event::Question { .. }
            | Event::Escalated { .. } => {}
        }
        if reload_list {
            self.list = self.source.conversations();
            self.listed = true;
        }
        if reload_shown {
            self.reload_shown();
            // A deleted conversation leaves the transcript for a new chat.
            if self.shown.is_none()
                && let Some(id) = &self.selected
                && !self.turns.contains_key(id)
                && !self.failures.contains_key(id)
                && !self.list.iter().any(|row| &row.id == id)
            {
                self.selected = None;
            }
        }
        self.tasks = self.source.tasks();
        true
    }

    /// Sends the composer's text: a new conversation, or the selected one
    /// continued. Nothing for an empty composer.
    pub(super) fn send(&mut self) -> bool {
        let question = self.composer.text.trim().to_owned();
        if question.is_empty() {
            return false;
        }
        let id = self.source.submit(Request {
            conversation: self.selected.clone(),
            text: question.clone(),
            origin: Origin::Chat,
            context: Context::default(),
        });
        if id.is_empty() {
            self.error =
                Some(tr("The assistant didn't take the message. Try again in a moment.").into());
            return true;
        }
        self.error = None;
        self.composer.clear();
        self.failures.remove(&id);
        self.turns.insert(
            id.clone(),
            Turn {
                question,
                origin: Origin::Chat,
                phase: Phase::Sending,
                steps: Vec::new(),
                answer: String::new(),
            },
        );
        self.delete_armed = None;
        if self.selected.as_deref() != Some(id.as_str()) {
            self.selected = Some(id);
            self.reload_shown();
        }
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
        match &mut self.source {
            Source::Fixture(store) => store.cancelled.extend(ids),
            Source::Live => crate::assistant::cancel_all(),
        }
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

    pub(super) fn set_agent_mode(&mut self, on: bool) {
        self.source.set_agent_mode(on);
        self.agent_mode = on;
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

/// Whether the pane is in the sidebar: Labs › Assistant is on, on macOS.
pub(super) fn available(ui: &Ui) -> bool {
    ui.platform.caps.os == super::platform::Os::Macos
        && crate::assistant::enabled(&ui.settings)
        && crate::labs::Feature::Assistant.managed().is_none()
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
    crate::assistant::subscribe(Box::new(move |event: &Event| {
        if let Ok(mut events) = inbox.lock() {
            events.push(event.clone());
        }
        waker();
    }));
}

/// Opens the window's Chats pane on `conversation` (or where it was), from
/// the menu bar, a `hex://chats` link, or the HUD's task stack.
pub(super) fn show(ui: &mut Ui, conversation: Option<ConversationId>, tx: &mut Tx) {
    if !available(ui) {
        return;
    }
    ui.select_pane(Pane::Chats, tx);
    ui.chats.view = View::Chats;
    if !ui.chats.listed {
        ui.chats.reload();
    }
    if let Some(id) = conversation {
        ui.chats.select(id);
    }
}

/// Opens the assistant's onboarding, and starts its checks.
pub(super) fn open_setup(ui: &mut Ui) {
    setup::open(ui);
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
    if !available(ui) {
        if ui.chats.setup.is_some() || ui.pane == Pane::Chats {
            // The Lab turned off: its onboarding closes, its pane leaves.
            setup::close(ui);
            if ui.pane == Pane::Chats {
                ui.select_pane(Pane::General, tx);
            }
            changed = true;
        }
        set_focus(ui, None);
        return changed;
    }
    let visible = ui.window.as_ref().is_none_or(rune::AppHandle::is_visible);
    let showing = visible && ui.pane == Pane::Chats && !ui.labs_open;
    if showing && !ui.chats.listed {
        ui.chats.reload();
        changed = true;
    }
    if !showing {
        // Read again when it next shows.
        ui.chats.listed = false;
    }
    let focus = if showing && ui.chats.view == View::Chats {
        ui.chats.selected.clone()
    } else {
        None
    };
    changed |= set_focus(ui, focus);
    changed |= setup::poll(ui, tx);
    changed |= settings::poll(ui, Instant::now());
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

/// Escape: disarms a delete. True when it did.
pub(super) fn escape(ui: &mut Ui) -> bool {
    ui.chats.delete_armed.take().is_some()
}

/// The pane: the setup notice, the running tasks, then the conversations
/// and the transcript, or the settings.
pub(super) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> (Vec<El>, El) {
    match ui.chats.view {
        View::Settings => (
            vec![setup_button(p)],
            super::scroll_body(p.k, "chats-settings-scroll", settings::page(ui, p, layer)),
        ),
        View::Chats => (header_actions(ui, p), list::body(ui, p)),
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

/// The Chats view's header: which agent answers, Agent mode, and New chat.
fn header_actions(ui: &Ui, p: Paint) -> Vec<El> {
    let backend = text(backend_name(ui.settings.assistant.backend))
        .key("chats-backend")
        .font(p.k.ty.small)
        .color(p.faint())
        .help(tr("The agent that answers. Change it in Settings"));
    let agent_mode = rune_kit::Switch::new(ui.chats.agent_mode)
        .label(tr("Agent mode"))
        .on_toggle(|ui: &mut Ui| {
            let on = !ui.chats.agent_mode;
            ui.chats.set_agent_mode(on);
        })
        .build(p.k)
        .key("agent-mode");
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
    vec![
        backend,
        row().key("agent-mode-slot").px(8.0).child(agent_mode),
        new_chat,
    ]
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
