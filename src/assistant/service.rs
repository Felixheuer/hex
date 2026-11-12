//! The assistant's task runner: takes requests, runs up to `max_parallel`
//! at once (each through `agent` on its backend), queues the rest, saves
//! conversations through `store`, owns Agent mode's on/off state and its
//! agent-desktop multi-agent session, and tells every subscriber what
//! happens. Callable from any thread.
//!
//! Each running task has its own thread, its own agent id in the desktop
//! session (its cursor), and its own colour (0..8). A conversation runs one
//! task at a time: a follow-up waits for the answer before it.
//!
//! OWNER: brain. The signatures here are the contract the Action shortcut,
//! the HUD, and the Chats pane build against.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::app_settings::AssistantSettings;

use super::agent::{self, Job, Outcome};
use super::provider::Control;
use super::store::{self, Store};
use super::types::{
    Conversation, ConversationId, ConversationSummary, Event, Origin, Request, TaskState,
    TaskStatus,
};

/// Called on a worker thread for every event; keep it quick (post to your
/// own thread or wake your loop).
pub type Subscriber = Box<dyn Fn(&Event) + Send + Sync>;

/// Each task colour's accent, by colour index: its cursor here, and its
/// outline and stack row in the HUD.
pub const COLOURS: [&str; 8] = [
    "#4299FF", "#FF6B4A", "#34C759", "#AF52DE", "#FF9F0A", "#FF375F", "#30B0C7", "#A2845E",
];

/// How long an Agent mode task waits for the desktop session to start.
const SESSION_WAIT: Duration = Duration::from_secs(8);

/// Queues `request` and returns its conversation's id at once (a new id when
/// `request.conversation` is `None`). A quick question (`Origin::ActionShortcut`)
/// cancels the previous quick question still running; Agent mode and Chat
/// tasks run beside each other.
pub fn submit(request: Request) -> ConversationId {
    runner().submit(request)
}

/// Stops one task: its agent process, its desktop actions, and its speech.
/// It ends with `Event::Cancelled`.
pub fn cancel(conversation: &str) {
    runner().cancel(conversation);
}

/// Stops every task.
pub fn cancel_all() {
    runner().cancel_all();
}

/// The tasks running or queued now, oldest first.
pub fn tasks() -> Vec<TaskStatus> {
    runner().tasks()
}

/// Whether any task is running or queued.
pub fn busy() -> bool {
    !runner().state().tasks.is_empty()
}

/// Whether Agent mode is on.
pub fn agent_mode() -> bool {
    runner().state().agent_mode
}

/// Turns Agent mode on or off (`Event::ModeChanged`). Turning it off lets
/// running tasks finish; their outlines and cursors stay until they do.
pub fn set_agent_mode(on: bool) {
    runner().set_agent_mode(on);
}

/// Calls `subscriber` for every later event, until the process ends.
/// Stops every task (the Stop all shortcut, the stack's and Chats' button,
/// the menu bar). Same as `cancel_all`.
pub fn stop_all() {
    cancel_all();
}

/// Whether Background work is running, waiting, or queued: the screen frame
/// shows while this is true.
pub fn background_busy() -> bool {
    agent_mode()
}

/// A hold of the Action shortcut just became real: get ready to answer in
/// `lane` (start its agent, warm its session) while the person speaks.
/// Called on the listener's thread; return at once.
pub fn prewarm(lane: super::types::Lane) {
    let _ = lane;
}

/// The person takes `conversation` over: its desktop actions wait, and its
/// window comes forward. `Event::Paused`.
pub fn take_over(conversation: &str) {
    let _ = conversation;
}

/// Gives `conversation` back after `take_over`; it looks again before acting.
/// `Event::Resumed`.
pub fn hand_back(conversation: &str) {
    let _ = conversation;
}

/// Continues a conversation Hex quit or crashed in the middle of
/// (`Outcome::Interrupted`), telling the agent to check what was already
/// done before going on.
pub fn resume_interrupted(conversation: &str) -> Option<ConversationId> {
    let _ = conversation;
    None
}

/// The plan of a running or finished conversation, if it made one.
pub fn plan(conversation: &str) -> Vec<crate::assistant::PlanStep> {
    let _ = conversation;
    Vec::new()
}

/// The question a task is waiting on (`TaskState::NeedsInput`), if any.
pub fn question(conversation: &str) -> Option<String> {
    let _ = conversation;
    None
}

pub fn subscribe(subscriber: Subscriber) {
    runner().subscribe(subscriber);
}

/// Every stored conversation, newest first.
pub fn conversations() -> Vec<ConversationSummary> {
    Store::open().map(|store| store.list()).unwrap_or_default()
}

pub fn conversation(id: &str) -> Option<Conversation> {
    Some(Store::open().ok()?.load(id)?.conversation)
}

pub fn delete(id: &str) {
    runner().cancel(id);
    if let Ok(store) = Store::open() {
        store.delete(id);
    }
    {
        let mut state = runner().state();
        if state.focus.as_deref() == Some(id) {
            state.focus = None;
        }
        if state
            .last_quick
            .as_ref()
            .is_some_and(|(quick, _)| quick == id)
        {
            state.last_quick = None;
        }
    }
    runner().emit(&Event::ConversationsChanged);
}

/// The conversation Chats shows, so a quick question continues it.
pub fn set_focus(id: Option<ConversationId>) {
    runner().state().focus = id;
}

/// The conversation the next quick question should continue: the one Chats
/// shows, else the last quick conversation answered within
/// `follow_up_seconds`, else `None` (start a new one). Agent mode never
/// continues one.
pub fn focus(follow_up_seconds: u32) -> Option<ConversationId> {
    runner().focus(follow_up_seconds)
}

/// Lets tasks run while Labs › Assistant is off, for `hex assistant`, which
/// a person runs on purpose.
pub fn allow_without_lab() {
    WITHOUT_LAB.store(true, Ordering::Release);
}

static WITHOUT_LAB: AtomicBool = AtomicBool::new(false);

/// Changes the settings every later task reads, in this process only (for
/// `hex assistant ask --backend …`).
pub fn adjust_settings(adjust: impl Fn(&mut AssistantSettings) + Send + Sync + 'static) {
    *ADJUST.lock().unwrap_or_else(|error| error.into_inner()) = Some(Box::new(adjust));
}

type Adjust = Box<dyn Fn(&mut AssistantSettings) + Send + Sync>;
static ADJUST: Mutex<Option<Adjust>> = Mutex::new(None);

fn runner() -> &'static Arc<Runner> {
    static RUNNER: OnceLock<Arc<Runner>> = OnceLock::new();
    RUNNER.get_or_init(|| Runner::new(Hooks::system()))
}

type Execute = dyn Fn(&Job, &Control, &mut dyn FnMut(Event)) -> Outcome + Send + Sync;
type StyleAgent = dyn Fn(&str, &str, u8) + Send + Sync;
type Speak = dyn Fn(&str, Option<&str>) + Send + Sync;

/// What the runner does in the world; tests replace each part.
pub(crate) struct Hooks {
    pub execute: Box<Execute>,
    /// Whether the Lab is on, and the assistant's settings, now.
    pub settings: Box<dyn Fn() -> (bool, AssistantSettings) + Send + Sync>,
    /// Starts an agent-desktop multi-agent cursor session; its id.
    pub start_session: Box<dyn Fn() -> Option<String> + Send + Sync>,
    pub end_session: Box<dyn Fn(&str) + Send + Sync>,
    /// Gives agent `agent_id` in `session` its colour.
    pub style_agent: Box<StyleAgent>,
    pub speak: Box<Speak>,
    pub stop_speaking: Box<dyn Fn() + Send + Sync>,
}

impl Hooks {
    /// The real ones: the backends, the saved settings, agent-desktop, and
    /// the macOS voice.
    fn system() -> Self {
        Self {
            execute: Box::new(agent::run),
            settings: Box::new(current_settings),
            start_session: Box::new(desktop_session::start),
            end_session: Box::new(desktop_session::end),
            style_agent: Box::new(desktop_session::style),
            speak: Box::new(|text, voice| {
                let voice = voice
                    .map(str::to_owned)
                    .or_else(|| super::speech::automatic_voice(text));
                super::speech::speak(text, voice.as_deref());
            }),
            stop_speaking: Box::new(super::speech::stop),
        }
    }
}

fn current_settings() -> (bool, AssistantSettings) {
    let (on, mut settings) = crate::app_settings::assistant_runtime().unwrap_or_else(|| {
        crate::app_settings::AppSettings::read()
            .map(|settings| (super::enabled(&settings), settings.assistant))
            .unwrap_or_default()
    });
    if let Some(adjust) = ADJUST
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        adjust(&mut settings);
    }
    (on || WITHOUT_LAB.load(Ordering::Acquire), settings)
}

/// The task runner.
pub(crate) struct Runner {
    hooks: Hooks,
    state: Mutex<State>,
    /// Signalled when the desktop session starts (or fails to).
    session_changed: Condvar,
    subscribers: Mutex<Vec<Arc<Subscriber>>>,
    next: AtomicU64,
}

#[derive(Default)]
pub(crate) struct State {
    tasks: Vec<Task>,
    agent_mode: bool,
    session: Session,
    focus: Option<ConversationId>,
    last_quick: Option<(ConversationId, Instant)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Session {
    #[default]
    Off,
    Starting,
    On(String),
}

struct Task {
    key: u64,
    request: Request,
    status: TaskStatus,
    control: Arc<Control>,
    agent_id: String,
    running: bool,
    announced: bool,
}

impl Runner {
    pub(crate) fn new(hooks: Hooks) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            state: Mutex::new(State::default()),
            session_changed: Condvar::new(),
            subscribers: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn subscribe(&self, subscriber: Subscriber) {
        self.subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Arc::new(subscriber));
    }

    /// Tells every subscriber, after noting a step or window on its task.
    fn emit(&self, event: &Event) {
        match event {
            Event::Step {
                conversation,
                description,
            } => {
                if let Some(task) = self.running_task(&mut self.state(), conversation) {
                    task.status.step = Some(description.clone());
                }
            }
            Event::Window {
                conversation,
                window,
            } => {
                if let Some(task) = self.running_task(&mut self.state(), conversation)
                    && !task.status.windows.contains(window)
                {
                    task.status.windows.push(window.clone());
                }
            }
            _ => {}
        }
        let subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        for subscriber in subscribers {
            subscriber(event);
        }
    }

    fn running_task<'a>(&self, state: &'a mut State, conversation: &str) -> Option<&'a mut Task> {
        state
            .tasks
            .iter_mut()
            .find(|task| task.running && task.status.conversation == conversation)
    }

    pub(crate) fn submit(self: &Arc<Self>, request: Request) -> ConversationId {
        let id = request.conversation.clone().unwrap_or_else(store::new_id);
        let (enabled, _) = (self.hooks.settings)();
        if !enabled {
            self.emit(&Event::Failed {
                conversation: id.clone(),
                message: "Turn on Labs › Assistant first.".into(),
            });
            return id;
        }
        let key = self.next.fetch_add(1, Ordering::Relaxed);
        let mut earlier_quick = Vec::new();
        {
            let mut state = self.state();
            if request.origin == Origin::ActionShortcut {
                earlier_quick = state
                    .tasks
                    .iter()
                    .filter(|task| task.request.origin == Origin::ActionShortcut)
                    .map(|task| task.key)
                    .collect();
            }
            let used: Vec<u8> = state.tasks.iter().map(|task| task.status.colour).collect();
            let colour = (0..8u8)
                .find(|colour| !used.contains(colour))
                .unwrap_or((key % 8) as u8);
            state.tasks.push(Task {
                key,
                status: TaskStatus {
                    conversation: id.clone(),
                    origin: request.origin,
                    title: store::shorten(&request.text, 60),
                    state: TaskState::Queued,
                    step: None,
                    windows: Vec::new(),
                    colour,
                },
                request: Request {
                    conversation: Some(id.clone()),
                    ..request
                },
                control: Control::new(),
                agent_id: format!("hex-{key}"),
                running: false,
                announced: false,
            });
        }
        for earlier in earlier_quick {
            self.cancel_task(earlier);
        }
        self.pump();
        id
    }

    /// Starts every queued task that has a free slot and whose conversation
    /// isn't already running; announces the rest as queued.
    fn pump(self: &Arc<Self>) {
        let (_, settings) = (self.hooks.settings)();
        let limit = settings.max_parallel.max(1) as usize;
        let mut start = Vec::new();
        let mut queued = Vec::new();
        {
            let mut state = self.state();
            let mut running = state.tasks.iter().filter(|task| task.running).count();
            let mut busy: Vec<ConversationId> = state
                .tasks
                .iter()
                .filter(|task| task.running)
                .map(|task| task.status.conversation.clone())
                .collect();
            for task in &mut state.tasks {
                if task.running {
                    continue;
                }
                if running < limit && !busy.contains(&task.status.conversation) {
                    task.running = true;
                    task.status.state = TaskState::Running;
                    running += 1;
                    busy.push(task.status.conversation.clone());
                    start.push(task.key);
                } else if !task.announced {
                    task.announced = true;
                    queued.push(Event::Queued {
                        conversation: task.status.conversation.clone(),
                        origin: task.request.origin,
                        question: task.request.text.clone(),
                    });
                }
            }
        }
        for event in &queued {
            self.emit(event);
        }
        for key in start {
            let runner = Arc::clone(self);
            std::thread::Builder::new()
                .name("assistant-task".into())
                .spawn(move || runner.run_task(key, settings_now(&runner)))
                .expect("the assistant can start a task thread");
        }

        fn settings_now(runner: &Runner) -> AssistantSettings {
            (runner.hooks.settings)().1
        }
    }

    fn run_task(self: &Arc<Self>, key: u64, settings: AssistantSettings) {
        let Some((request, control, agent_id, colour)) = self
            .state()
            .tasks
            .iter()
            .find(|task| task.key == key)
            .map(|task| {
                (
                    task.request.clone(),
                    Arc::clone(&task.control),
                    task.agent_id.clone(),
                    task.status.colour,
                )
            })
        else {
            return;
        };
        let conversation = request.conversation.clone().unwrap_or_default();
        self.emit(&Event::Started {
            conversation: conversation.clone(),
            origin: request.origin,
            question: request.text.clone(),
        });
        let desktop_session = if settings.control_desktop {
            self.session_for(request.origin)
        } else {
            None
        };
        if let Some(session) = &desktop_session {
            (self.hooks.style_agent)(session, &agent_id, colour);
        }
        let job = Job {
            conversation: conversation.clone(),
            request: request.clone(),
            settings: settings.clone(),
            agent_id,
            desktop_session,
        };
        let outcome = if control.cancelled() {
            Outcome::Cancelled
        } else {
            (self.hooks.execute)(&job, &control, &mut |event| self.emit(&event))
        };
        let outcome = if control.cancelled() {
            Outcome::Cancelled
        } else {
            outcome
        };
        let end_session = {
            let mut state = self.state();
            state.tasks.retain(|task| task.key != key);
            if request.origin == Origin::ActionShortcut && matches!(outcome, Outcome::Answer(_)) {
                state.last_quick = Some((conversation.clone(), Instant::now()));
            }
            self.session_to_end(&mut state)
        };
        if let Some(session) = end_session {
            (self.hooks.end_session)(&session);
        }
        match outcome {
            Outcome::Answer(text) => {
                if request.origin == Origin::ActionShortcut && settings.speak_answers {
                    (self.hooks.speak)(&text, settings.voice.as_deref());
                }
                self.emit(&Event::Answer {
                    conversation,
                    origin: request.origin,
                    text,
                });
            }
            Outcome::Failed(message) => self.emit(&Event::Failed {
                conversation,
                message,
            }),
            Outcome::Cancelled => self.emit(&Event::Cancelled { conversation }),
        }
        self.pump();
    }

    /// The desktop session a task joins: Agent mode's, waiting a moment
    /// while it starts.
    fn session_for(&self, origin: Origin) -> Option<String> {
        let started = Instant::now();
        let mut state = self.state();
        loop {
            match &state.session {
                Session::On(id) => return Some(id.clone()),
                Session::Off => return None,
                Session::Starting if origin != Origin::Background => return None,
                Session::Starting => {
                    let left = SESSION_WAIT.saturating_sub(started.elapsed());
                    if left.is_zero() {
                        return None;
                    }
                    state = self
                        .session_changed
                        .wait_timeout(state, left)
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                }
            }
        }
    }

    /// The session to end now that Agent mode is off and no task remains.
    fn session_to_end(&self, state: &mut State) -> Option<String> {
        if state.agent_mode || !state.tasks.is_empty() {
            return None;
        }
        match std::mem::take(&mut state.session) {
            Session::On(id) => Some(id),
            other => {
                state.session = other;
                None
            }
        }
    }

    pub(crate) fn set_agent_mode(self: &Arc<Self>, on: bool) {
        let (enabled, settings) = (self.hooks.settings)();
        if on && !enabled {
            return;
        }
        let mut start = false;
        let end = {
            let mut state = self.state();
            if state.agent_mode == on {
                return;
            }
            state.agent_mode = on;
            if on && settings.control_desktop && state.session == Session::Off {
                state.session = Session::Starting;
                start = true;
            }
            self.session_to_end(&mut state)
        };
        if let Some(session) = end {
            (self.hooks.end_session)(&session);
        }
        if start {
            let runner = Arc::clone(self);
            std::thread::spawn(move || {
                let started = (runner.hooks.start_session)();
                let end = {
                    let mut state = runner.state();
                    state.session = started.map_or(Session::Off, Session::On);
                    runner.session_changed.notify_all();
                    runner.session_to_end(&mut state)
                };
                if let Some(session) = end {
                    (runner.hooks.end_session)(&session);
                }
            });
        }
        self.emit(&Event::ModeChanged { on });
    }

    pub(crate) fn cancel(&self, conversation: &str) {
        let keys: Vec<u64> = self
            .state()
            .tasks
            .iter()
            .filter(|task| task.status.conversation == conversation)
            .map(|task| task.key)
            .collect();
        for key in keys {
            self.cancel_task(key);
        }
    }

    pub(crate) fn cancel_all(&self) {
        let keys: Vec<u64> = self.state().tasks.iter().map(|task| task.key).collect();
        for key in keys {
            self.cancel_task(key);
        }
        (self.hooks.stop_speaking)();
    }

    /// Stops a running task (it ends with `Cancelled` on its own thread), or
    /// drops a queued one and says so.
    fn cancel_task(&self, key: u64) {
        let (dropped, quick) = {
            let mut state = self.state();
            let Some(index) = state.tasks.iter().position(|task| task.key == key) else {
                return;
            };
            let task = &state.tasks[index];
            let quick = task.request.origin == Origin::ActionShortcut;
            task.control.cancel();
            if task.running {
                (None, quick)
            } else {
                let task = state.tasks.remove(index);
                (Some(task.status.conversation), quick)
            }
        };
        if quick {
            (self.hooks.stop_speaking)();
        }
        if let Some(conversation) = dropped {
            self.emit(&Event::Cancelled { conversation });
        }
    }

    pub(crate) fn tasks(&self) -> Vec<TaskStatus> {
        self.state()
            .tasks
            .iter()
            .map(|task| task.status.clone())
            .collect()
    }

    pub(crate) fn focus(&self, follow_up_seconds: u32) -> Option<ConversationId> {
        let state = self.state();
        if state.agent_mode {
            return None;
        }
        if let Some(focus) = &state.focus {
            return Some(focus.clone());
        }
        let (id, answered) = state.last_quick.as_ref()?;
        (follow_up_seconds > 0
            && answered.elapsed() <= Duration::from_secs(follow_up_seconds.into()))
        .then(|| id.clone())
    }
}

/// Agent mode's agent-desktop session: one multi-agent cursor session, every
/// task its own agent (and cursor) in it.
mod desktop_session {
    use std::process::Stdio;

    use serde_json::Value;

    fn agent_desktop(args: &[&str]) -> Option<Value> {
        let executable = crate::assistant::provider::agent_desktop()?;
        let output = crate::assistant::backends::agent_command(executable)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        serde_json::from_slice(&output.stdout).ok()
    }

    /// `agent-desktop session start --cursor --multi-agent`; its id.
    pub(super) fn start() -> Option<String> {
        let reply = agent_desktop(&[
            "session",
            "start",
            "--cursor",
            "--multi-agent",
            "--name",
            "Hex Agent mode",
        ]);
        let id = reply
            .as_ref()
            .and_then(|reply| reply.pointer("/data/session_id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if id.is_none() {
            tracing::warn!(
                ?reply,
                "agent-desktop didn't start a session for Agent mode"
            );
        }
        id
    }

    pub(super) fn end(session: &str) {
        let _ = agent_desktop(&["session", "end", session]);
    }

    /// Gives the agent's cursor its task colour.
    pub(super) fn style(session: &str, agent_id: &str, colour: u8) {
        let accent = super::COLOURS[usize::from(colour) % super::COLOURS.len()];
        let _ = agent_desktop(&[
            "--session",
            session,
            "--agent-id",
            agent_id,
            "cursor-overlay",
            "enable",
            "--accent",
            accent,
        ]);
    }
}

#[cfg(test)]
mod tests;
