//! The assistant's task runner: takes requests, runs them (each through
//! `agent` on its backend) in their lanes, saves conversations through
//! `store`, owns the screen frame's state and the agent-desktop multi-agent
//! session around Background work, and tells every subscriber what happens.
//! Callable from any thread.
//!
//! - **Lanes.** At most one Here task (`Origin::ActionShortcut`) runs: a new
//!   one stops the last, unless it speaks into it. Background and Chat tasks
//!   run beside each other, up to `max_parallel`; later ones queue.
//! - **Steering.** A request for a conversation that is running (or that
//!   names a running task, `context::route`) stops its current turn and goes
//!   on in the same conversation with the new instruction.
//! - **Context.** Every new conversation's first prompt lists the tasks
//!   running and the conversations just finished (`context::digest`).
//! - **Frame.** `background_busy` is true while any Background or Chat task
//!   is queued, running, waiting, or paused (`Event::ModeChanged`).
//! - **Take over.** `take_over` pauses a task's desktop actions through the
//!   guard and brings its window forward; `hand_back` resumes it.
//! - **Durability.** A conversation is `Running` on disk while it runs; one
//!   left so by a Hex that quit reads `Interrupted`, and `resume_interrupted`
//!   continues it. A task stops after `task_timeout_minutes`; one that hits
//!   its agent's usage limit waits for the reset and retries
//!   (`retry_on_limit`). Every end releases the task's window leases and
//!   browser.
//! - **Speed.** `prewarm` starts the backend while the person still speaks,
//!   and a Here answer is read aloud sentence by sentence as it streams.
//!
//! Each running task has its own thread, its own agent id in the desktop
//! session (its cursor), and its own colour (0..8). A conversation runs one
//! task at a time: a follow-up waits for the answer before it.
//!
//! OWNER: brain. The signatures here are the contract the Action shortcut,
//! the HUD, and the Chats pane build against.

mod context;
mod monitor;
pub mod remote;
mod voice;

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::app_settings::{AssistantBackend, AssistantSettings};

use super::agent::{self, Job, JobKind, Outcome};
use super::provider::{self, Control};
use super::store::{self, Store};
use super::types::{
    Context, Conversation, ConversationId, ConversationSummary, Event, Lane, Origin,
    Outcome as Ended, Request, TaskState, TaskStatus, TaskWindow,
};

/// Called on a worker thread for every event; keep it quick (post to your
/// own thread or wake your loop).
pub type Subscriber = Box<dyn Fn(&Event) + Send + Sync>;

/// Each task colour's accent, by colour index: its cursor here, and its
/// outline and stack row in the HUD.
pub const COLOURS: [&str; 8] = [
    "#4299FF", "#FF6B4A", "#34C759", "#AF52DE", "#FF9F0A", "#FF375F", "#30B0C7", "#A2845E",
];

/// How long a Background task waits for the desktop session to start.
const SESSION_WAIT: Duration = Duration::from_secs(8);
/// How long a backend started ahead is kept for its question.
const PREWARM_FOR_MS: u64 = 60_000;
/// Retries after a limit that named no reset time, before giving up.
const BLIND_RETRIES: u32 = 8;
/// A task's last steps kept for steering.
const STEPS_KEPT: usize = 4;

/// Queues `request` and returns its conversation's id at once (a new id when
/// `request.conversation` is `None`).
///
/// - A request for a conversation that is running **steers** it: its current
///   turn stops, and it goes on in the same conversation (same backend
///   session, windows, and browser) told "The person interrupted you while
///   you were doing X … Their new instruction: …". The stopped turn ends
///   without a final event; the steered run then reports `Started` and ends
///   with the conversation's one `Answer`/`Failed`/`Cancelled`.
/// - A new request (`conversation: None`) that names one running task by
///   its title or apps ("bei der Mail-Aufgabe …") steers that task.
/// - Otherwise a Here request stops any other Here task; Background and
///   Chat tasks run beside each other.
pub fn submit(request: Request) -> ConversationId {
    runner().submit(request)
}

/// A hold of the Action shortcut just became real: starts the backend for
/// the next `lane` request while the person is still speaking. Returns at
/// once; for the Here lane it reads the window in front itself (a few
/// milliseconds, off the caller's thread). The next matching `submit` (same
/// lane, same window, same follow-up conversation) finds the process and its
/// MCP servers up. Cheap to call again; does nothing for backends that can't
/// start ahead (only Claude Code can) or while the assistant is off.
pub fn prewarm(lane: Lane) {
    std::thread::Builder::new()
        .name("assistant-prewarm".into())
        .spawn(move || {
            let window = match lane {
                Lane::Here => super::computer::quick_context().window,
                Lane::Background => None,
            };
            runner().prewarm(lane, window);
        })
        .ok();
}

/// `prewarm` for a window the caller already knows (`hex assistant ask`).
pub fn prewarm_for(lane: Lane, window: Option<TaskWindow>) {
    runner().prewarm(lane, window);
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

/// The screen frame's state, as `background_busy` (Agent mode is gone: the
/// frame follows Background work).
pub fn agent_mode() -> bool {
    background_busy()
}

/// Kept for callers of the old Agent mode switch: turning it off stops every
/// task; turning it on does nothing (Background work turns the frame on).
pub fn set_agent_mode(on: bool) {
    if !on {
        stop_all();
    }
}

/// Stops every task (the Stop all shortcut, the stack's and Chats' button,
/// the menu bar), and ends backends started ahead.
pub fn stop_all() {
    cancel_all();
    provider::cool_down();
}

/// Whether Background work is running, waiting, paused, or queued: the
/// screen frame shows while this is true.
pub fn background_busy() -> bool {
    runner().state().background_on
}

/// The person takes `conversation` over: its desktop actions wait (through
/// the guard), and its window comes forward. `Event::Paused`.
pub fn take_over(conversation: &str) {
    runner().take_over(conversation);
}

/// Gives `conversation` back after `take_over`; it looks again before acting.
/// `Event::Resumed`.
pub fn hand_back(conversation: &str) {
    runner().hand_back(conversation);
}

/// Continues a conversation Hex quit or crashed in the middle of
/// (`Outcome::Interrupted`), telling the agent to check what was already
/// done before going on. A Here conversation goes on in the Background lane:
/// its window may be gone. `None` when it wasn't interrupted.
pub fn resume_interrupted(conversation: &str) -> Option<ConversationId> {
    runner().resume_interrupted(conversation)
}

/// Calls `subscriber` for every later event, until the process ends.
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

/// The conversation the next Here request should continue: a Here task
/// running now (it is steered), else the one Chats shows, else the last Here
/// conversation answered within `follow_up_seconds`, else `None` (start a
/// new one).
pub fn focus(follow_up_seconds: u32) -> Option<ConversationId> {
    runner().focus(follow_up_seconds)
}

/// Lets tasks run while the assistant is off or not set up, for `hex
/// assistant`, which a person runs on purpose.
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
    RUNNER.get_or_init(|| {
        // Conversations a Hex that quit left running are interrupted, and
        // the guard forgets what ended tasks left (only in the app: a `hex
        // assistant` command must not clear the app's pauses).
        std::thread::spawn(|| {
            if !WITHOUT_LAB.load(Ordering::Acquire) && current_settings().0 {
                super::guard::startup();
            }
            if let Ok(store) = Store::open() {
                let recovered = store.recover_interrupted();
                if !recovered.is_empty() {
                    tracing::info!(count = recovered.len(), "assistant tasks were interrupted");
                }
            }
        });
        Runner::new(Hooks::system())
    })
}

type Execute = dyn Fn(&Job, &Control, &mut dyn FnMut(Event)) -> Outcome + Send + Sync;
type StyleAgent = dyn Fn(&str, &str, u8) + Send + Sync;
type Speak = dyn Fn(&str, Option<&str>) + Send + Sync;
type Hook<T> = Box<dyn Fn() -> T + Send + Sync>;
type TaskHook = Box<dyn Fn(&str) + Send + Sync>;

/// What the runner does in the world; tests replace each part.
pub(crate) struct Hooks {
    pub execute: Box<Execute>,
    /// Whether the assistant is on, and its settings, now.
    pub settings: Hook<(bool, AssistantSettings)>,
    /// Starts an agent-desktop multi-agent cursor session; its id.
    pub start_session: Hook<Option<String>>,
    pub end_session: TaskHook,
    /// Gives agent `agent_id` in `session` its colour.
    pub style_agent: Box<StyleAgent>,
    /// Starts reading text aloud with a voice (returns at once).
    pub speak: Box<Speak>,
    /// Whether something is being read aloud.
    pub speaking: Hook<bool>,
    /// The voice for an answer, from its first sentence, when none is set.
    pub pick_voice: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
    pub stop_speaking: Hook<()>,
    /// Unix ms now, and a pause: the runner's clock.
    pub now_ms: Hook<u64>,
    pub sleep: Box<dyn Fn(Duration) + Send + Sync>,
    /// Pauses or resumes a task's desktop actions (`guard::set_paused`).
    pub pause: Box<dyn Fn(&str, bool) + Send + Sync>,
    /// A task ended: its window leases and browser go (`guard::release_all`,
    /// `browser::close`).
    pub release: TaskHook,
    /// The task moving the real pointer now (`guard::hands_off`).
    pub hands_off: Hook<Option<ConversationId>>,
    /// Brings a task's window forward (taken over).
    pub bring_forward: Box<dyn Fn(&str, &[TaskWindow]) + Send + Sync>,
    /// Records how a conversation's request ended when its run didn't.
    pub settle: Box<dyn Fn(&str, Ended, &str) + Send + Sync>,
    /// The latest stored conversations, newest first.
    pub recent: Hook<Vec<ConversationSummary>>,
    /// An interrupted conversation's lane and last question.
    pub interrupted: Box<dyn Fn(&str) -> Option<(Origin, String)> + Send + Sync>,
    /// Starts a job's backend ahead of its question.
    pub prewarm: Box<dyn Fn(&Job) + Send + Sync>,
    /// Requests from other Hex processes (`remote`).
    pub requests: Hook<Vec<(String, remote::Verb)>>,
}

impl Hooks {
    /// The real ones: the backends, the saved settings, agent-desktop, the
    /// guard, the browser, the store, and the macOS voice.
    fn system() -> Self {
        let seen = Mutex::new(std::collections::HashSet::new());
        Self {
            execute: Box::new(agent::run),
            settings: Box::new(current_settings),
            start_session: Box::new(desktop_session::start),
            end_session: Box::new(desktop_session::end),
            style_agent: Box::new(desktop_session::style),
            speak: Box::new(super::speech::speak),
            speaking: Box::new(super::speech::speaking),
            pick_voice: Box::new(super::speech::automatic_voice),
            stop_speaking: Box::new(super::speech::stop),
            now_ms: Box::new(store::now_ms),
            sleep: Box::new(std::thread::sleep),
            pause: Box::new(super::guard::set_paused),
            release: Box::new(|task| {
                super::guard::release_all(task);
                // Closing the browser takes a moment; the task has ended.
                let task = task.to_owned();
                std::thread::spawn(move || super::browser::close(&task));
            }),
            hands_off: Box::new(super::guard::hands_off),
            bring_forward: Box::new(desktop_session::bring_forward),
            settle: Box::new(|id, outcome, message| {
                if let Ok(store) = Store::open() {
                    store.settle(id, outcome, Some(message));
                }
            }),
            recent: Box::new(|| {
                Store::open()
                    .map(|store| store.list().into_iter().take(12).collect())
                    .unwrap_or_default()
            }),
            interrupted: Box::new(|id| {
                let stored = Store::open().ok()?.load(id)?;
                (stored.current_outcome() == Ended::Interrupted).then_some(())?;
                Some((stored.conversation.origin, stored.last_question()?))
            }),
            prewarm: Box::new(agent::prewarm),
            requests: Box::new(move || {
                remote::take_new(&mut seen.lock().unwrap_or_else(|error| error.into_inner()))
            }),
        }
    }
}

/// The settings as applied, or as saved (read at most every two seconds)
/// for a `hex` command that never applies them.
fn current_settings() -> (bool, AssistantSettings) {
    static READ: Mutex<Option<(Instant, (bool, AssistantSettings))>> = Mutex::new(None);
    let (on, mut settings) = crate::app_settings::assistant_runtime().unwrap_or_else(|| {
        let mut read = READ.lock().unwrap_or_else(|error| error.into_inner());
        match read.as_ref() {
            Some((at, settings)) if at.elapsed() < Duration::from_secs(2) => settings.clone(),
            _ => {
                let settings = crate::app_settings::AppSettings::read()
                    .map(|settings| (super::enabled(&settings), settings.assistant))
                    .unwrap_or_default();
                *read = Some((Instant::now(), settings.clone()));
                settings
            }
        }
    });
    if let Some(adjust) = ADJUST
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        adjust(&mut settings);
    }
    let without_lab = WITHOUT_LAB.load(Ordering::Acquire);
    if without_lab {
        settings.setup_done = true;
    }
    (on || without_lab, settings)
}

/// The task runner.
pub(crate) struct Runner {
    hooks: Hooks,
    state: Mutex<State>,
    /// Signalled when the desktop session starts (or fails to).
    session_changed: Condvar,
    subscribers: Mutex<Vec<Arc<Subscriber>>>,
    next: AtomicU64,
    voice: voice::Voice,
}

#[derive(Default)]
pub(crate) struct State {
    tasks: Vec<Task>,
    /// The screen frame shows (`background_busy`).
    background_on: bool,
    session: Session,
    focus: Option<ConversationId>,
    /// The last Here conversation answered, and when (runner clock).
    last_quick: Option<(ConversationId, u64)>,
    /// The monitor thread runs (`monitor`).
    monitor: bool,
    /// The task the HUD shows "Hands off" for.
    hands_off: Option<ConversationId>,
    /// A backend started ahead, waiting for its request.
    warm: Option<Warm>,
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Session {
    #[default]
    Off,
    Starting,
    On(String),
}

/// What `prewarm` reserved for the next request.
#[derive(Clone, Debug)]
struct Warm {
    lane: Lane,
    /// The conversation it continues (`None`: a new one).
    conversation: Option<ConversationId>,
    /// The id the new conversation gets, and its task key (agent id).
    id: ConversationId,
    key: u64,
    window: Option<TaskWindow>,
    at_ms: u64,
}

struct Task {
    key: u64,
    request: Request,
    status: TaskStatus,
    control: Arc<Control>,
    agent_id: String,
    running: bool,
    announced: bool,
    kind: JobKind,
    /// It started a conversation (its prompt gets the digest).
    new_conversation: bool,
    /// Its answer is read aloud as it streams.
    speak: bool,
    /// Its last few steps, for steering.
    steps: VecDeque<String>,
    /// When it times out (runner clock), while running.
    deadline_ms: Option<u64>,
    /// Since when it has been paused or waiting (not counted to its time).
    held_since: Option<u64>,
    /// Its state before it was taken over.
    before_pause: TaskState,
}

impl Task {
    fn lane(&self) -> Lane {
        self.request.origin.lane()
    }
}

impl Runner {
    pub(crate) fn new(hooks: Hooks) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            state: Mutex::new(State::default()),
            session_changed: Condvar::new(),
            subscribers: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            voice: voice::Voice::default(),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn now(&self) -> u64 {
        (self.hooks.now_ms)()
    }

    pub(crate) fn subscribe(&self, subscriber: Subscriber) {
        self.subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Arc::new(subscriber));
    }

    /// Tells every subscriber, after noting a step or window on its task and
    /// reading a Here answer aloud as it streams.
    fn emit(self: &Arc<Self>, event: &Event) {
        match event {
            Event::Step {
                conversation,
                description,
            } => {
                if let Some(task) = running_task(&mut self.state(), conversation) {
                    task.status.step = Some(description.clone());
                    task.steps.push_back(description.clone());
                    while task.steps.len() > STEPS_KEPT {
                        task.steps.pop_front();
                    }
                }
            }
            Event::Window {
                conversation,
                window,
            } => {
                if let Some(task) = running_task(&mut self.state(), conversation)
                    && !task.status.windows.contains(window)
                {
                    task.status.windows.push(window.clone());
                }
            }
            Event::AnswerDelta { conversation, text } => {
                let speak = running_task(&mut self.state(), conversation)
                    .is_some_and(|task| task.speak && !task.control.cancelled());
                if speak {
                    self.voice.feed(self, conversation, text);
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

    pub(crate) fn submit(self: &Arc<Self>, request: Request) -> ConversationId {
        self.submit_as(request, JobKind::Ask)
    }

    fn submit_as(self: &Arc<Self>, mut request: Request, mut kind: JobKind) -> ConversationId {
        let (enabled, settings) = (self.hooks.settings)();
        let refuse = |message: &str| {
            let id = request.conversation.clone().unwrap_or_else(store::new_id);
            self.emit(&Event::Failed {
                conversation: id.clone(),
                message: message.into(),
            });
            id
        };
        if !enabled {
            return refuse("Turn on the assistant first.");
        }
        if !settings.setup_done {
            return refuse("Finish setting up the assistant first (Settings › Assistant).");
        }
        let now = self.now();
        let mut stop = Vec::new();
        let id = {
            let mut state = self.state();
            if kind == JobKind::Ask && request.conversation.is_none() {
                let running: Vec<TaskStatus> = state
                    .tasks
                    .iter()
                    .filter(|task| task.running)
                    .map(|task| task.status.clone())
                    .collect();
                request.conversation = context::route(&request.text, &running);
            }
            let new_conversation = request.conversation.is_none();
            let mut id = request.conversation.clone().unwrap_or_else(store::new_id);
            let mut key = None;
            if let Some(warm) = state.warm.take()
                && kind == JobKind::Ask
                && warm.lane == request.origin.lane()
                && warm.conversation == request.conversation
                && warm.window == request.context.window
                && now.saturating_sub(warm.at_ms) < PREWARM_FOR_MS
            {
                if new_conversation {
                    id = warm.id;
                }
                key = Some(warm.key);
            }
            if kind == JobKind::Ask
                && let Some(target) = state.tasks.iter_mut().find(|task| {
                    task.running && task.status.conversation == id && !task.control.steered()
                })
            {
                kind = JobKind::Steer {
                    doing: target.status.step.clone(),
                    steps: target.steps.iter().cloned().collect(),
                };
                request.origin = target.request.origin;
                request.context = Context {
                    screenshot: None,
                    ..target.request.context.clone()
                };
                target.control.steer();
            }
            if request.origin.lane() == Lane::Here {
                stop = state
                    .tasks
                    .iter()
                    .filter(|task| task.lane() == Lane::Here && task.status.conversation != id)
                    .map(|task| task.key)
                    .collect();
            }
            let key = key.unwrap_or_else(|| self.next.fetch_add(1, Ordering::Relaxed));
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
                kind,
                new_conversation,
                speak: false,
                steps: VecDeque::new(),
                deadline_ms: None,
                held_since: None,
                before_pause: TaskState::Running,
            });
            id
        };
        for key in stop {
            self.cancel_task(key);
        }
        self.refresh_mode();
        self.pump();
        id
    }

    /// Starts every queued task that has a free slot in its lane and whose
    /// conversation isn't already running; announces the rest as queued.
    fn pump(self: &Arc<Self>) {
        let (_, settings) = (self.hooks.settings)();
        let limit = settings.max_parallel.max(1) as usize;
        let mut start = Vec::new();
        let mut queued = Vec::new();
        let mut monitor = false;
        {
            let mut state = self.state();
            let running = |lane: Lane, state: &State| {
                state
                    .tasks
                    .iter()
                    .filter(|task| task.running && task.lane() == lane)
                    .count()
            };
            let mut background = running(Lane::Background, &state);
            let mut here = running(Lane::Here, &state);
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
                let free = match task.lane() {
                    Lane::Here => here == 0,
                    Lane::Background => background < limit,
                };
                if free && !busy.contains(&task.status.conversation) {
                    task.running = true;
                    task.status.state = TaskState::Running;
                    match task.lane() {
                        Lane::Here => here += 1,
                        Lane::Background => background += 1,
                    }
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
            if !start.is_empty() && !state.monitor {
                state.monitor = true;
                monitor = true;
            }
        }
        for event in &queued {
            self.emit(event);
        }
        if monitor {
            let runner = Arc::clone(self);
            std::thread::Builder::new()
                .name("assistant-monitor".into())
                .spawn(move || runner.monitor())
                .expect("the assistant can start its monitor");
        }
        for key in start {
            let runner = Arc::clone(self);
            std::thread::Builder::new()
                .name("assistant-task".into())
                .spawn(move || runner.run_task(key))
                .expect("the assistant can start a task thread");
        }
    }

    fn run_task(self: &Arc<Self>, key: u64) {
        let (_, settings) = (self.hooks.settings)();
        let now = self.now();
        let Some((request, control, agent_id, colour, kind, new_conversation)) = ({
            let mut state = self.state();
            state.tasks.iter_mut().find(|task| task.key == key).map(|task| {
                task.speak = task.lane() == Lane::Here && settings.speak_answers;
                task.deadline_ms = (settings.task_timeout_minutes > 0)
                    .then(|| now + u64::from(settings.task_timeout_minutes) * 60_000);
                (
                    task.request.clone(),
                    Arc::clone(&task.control),
                    task.agent_id.clone(),
                    task.status.colour,
                    task.kind.clone(),
                    task.new_conversation,
                )
            })
        }) else {
            return;
        };
        let conversation = request.conversation.clone().unwrap_or_default();
        let lane = request.origin.lane();
        self.emit(&Event::Started {
            conversation: conversation.clone(),
            origin: request.origin,
            question: request.text.clone(),
        });
        let desktop_session = if settings.control_desktop {
            self.session_for(lane)
        } else {
            None
        };
        if let Some(session) = &desktop_session {
            (self.hooks.style_agent)(session, &agent_id, colour);
        }
        let digest = (new_conversation && kind == JobKind::Ask)
            .then(|| self.digest(&conversation))
            .flatten();
        let mut job = Job {
            conversation: conversation.clone(),
            request: request.clone(),
            settings: settings.clone(),
            agent_id,
            desktop_session,
            kind,
            digest,
        };
        let mut attempts = 0;
        let outcome = loop {
            let outcome = if control.cancelled() {
                Outcome::Cancelled
            } else {
                std::panic::catch_unwind(AssertUnwindSafe(|| {
                    (self.hooks.execute)(&job, &control, &mut |event| self.emit(&event))
                }))
                .unwrap_or_else(|_| {
                    tracing::error!("an assistant task panicked");
                    Outcome::Failed("The task stopped unexpectedly.".into())
                })
            };
            let Outcome::Limited { reason, until_ms } = outcome else {
                break outcome;
            };
            attempts += 1;
            if until_ms.is_none() && attempts > BLIND_RETRIES {
                break Outcome::Failed(format!("{reason} (still limited after {attempts} tries)"));
            }
            let until = until_ms.unwrap_or_else(|| self.now() + backoff(attempts));
            self.wait_out(key, &conversation, &control, until, reason);
            if control.cancelled() {
                break Outcome::Cancelled;
            }
            job.kind = JobKind::AfterLimit;
            job.digest = None;
        };
        let outcome = if control.steered() {
            Outcome::Steered
        } else if let (Some(reason), Outcome::Cancelled) = (control.failure(), &outcome) {
            Outcome::Failed(reason)
        } else if control.cancelled() {
            Outcome::Cancelled
        } else {
            outcome
        };
        let steered = outcome == Outcome::Steered;
        let now = self.now();
        let (paused, hands_off, end_session) = {
            let mut state = self.state();
            let paused = state
                .tasks
                .iter()
                .find(|task| task.key == key)
                .is_some_and(|task| task.status.state == TaskState::Paused);
            state.tasks.retain(|task| task.key != key);
            if lane == Lane::Here && matches!(outcome, Outcome::Answer(_)) {
                state.last_quick = Some((conversation.clone(), now));
            }
            let hands_off = (state.hands_off.as_deref() == Some(conversation.as_str())
                && !steered)
                .then(|| state.hands_off.take())
                .flatten();
            (paused, hands_off, self.session_to_end(&mut state))
        };
        if let Some(conversation) = hands_off {
            self.emit(&Event::HandsOff {
                conversation,
                on: false,
            });
        }
        if !steered {
            if paused {
                (self.hooks.pause)(&conversation, false);
            }
            (self.hooks.release)(&conversation);
            let (ended, message) = match &outcome {
                Outcome::Answer(_) => (Ended::Answered, ""),
                Outcome::Failed(message) => (Ended::Failed, message.as_str()),
                _ => (Ended::Cancelled, "Stopped."),
            };
            (self.hooks.settle)(&conversation, ended, message);
        }
        if let Some(session) = end_session {
            (self.hooks.end_session)(&session);
        }
        match outcome {
            Outcome::Answer(text) => {
                if lane == Lane::Here && settings.speak_answers {
                    self.voice.finish(self, &conversation, &text);
                }
                self.emit(&Event::Answer {
                    conversation,
                    origin: request.origin,
                    text,
                });
            }
            Outcome::Failed(message) => {
                self.voice.drop_answer(&conversation);
                self.emit(&Event::Failed {
                    conversation,
                    message,
                });
            }
            Outcome::Cancelled => {
                self.voice.drop_answer(&conversation);
                self.emit(&Event::Cancelled { conversation });
            }
            Outcome::Steered | Outcome::Limited { .. } => {
                self.voice.drop_answer(&conversation);
            }
        }
        self.refresh_mode();
        self.pump();
    }

    /// What else is going on, for a new conversation's first prompt.
    fn digest(&self, conversation: &str) -> Option<String> {
        let running: Vec<TaskStatus> = self
            .state()
            .tasks
            .iter()
            .filter(|task| task.status.conversation != conversation)
            .map(|task| task.status.clone())
            .collect();
        let recent: Vec<ConversationSummary> = (self.hooks.recent)()
            .into_iter()
            .filter(|summary| summary.id != conversation)
            .collect();
        context::digest(&running, &recent, self.now())
    }

    /// The desktop session a task joins: the Background work's, waiting a
    /// moment while it starts (a Here task doesn't wait).
    fn session_for(&self, lane: Lane) -> Option<String> {
        let started = Instant::now();
        let mut state = self.state();
        loop {
            match &state.session {
                Session::On(id) => return Some(id.clone()),
                Session::Off => return None,
                Session::Starting if lane == Lane::Here => return None,
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

    /// The session to end now that no Background work is left and no task
    /// remains.
    fn session_to_end(&self, state: &mut State) -> Option<String> {
        if state.background_on || !state.tasks.is_empty() {
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

    /// Turns the frame on or off as Background work starts or ends, with the
    /// desktop session around it (`Event::ModeChanged`).
    fn refresh_mode(self: &Arc<Self>) {
        let (_, settings) = (self.hooks.settings)();
        let mut start = false;
        let (on, end) = {
            let mut state = self.state();
            let busy = state
                .tasks
                .iter()
                .any(|task| task.lane() == Lane::Background);
            if busy == state.background_on {
                return;
            }
            state.background_on = busy;
            if busy && settings.control_desktop && state.session == Session::Off {
                state.session = Session::Starting;
                start = true;
            }
            (busy, self.session_to_end(&mut state))
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

    pub(crate) fn prewarm(self: &Arc<Self>, lane: Lane, window: Option<TaskWindow>) {
        let (enabled, settings) = (self.hooks.settings)();
        if !enabled || !settings.setup_done || settings.backend != AssistantBackend::ClaudeCode {
            return;
        }
        let now = self.now();
        let (conversation, id, key, session) = {
            let mut state = self.state();
            let conversation = match lane {
                Lane::Here => focus_in(&state, settings.follow_up_seconds, now),
                Lane::Background => None,
            };
            // Speaking into a running task steers it: nothing to start.
            if conversation.as_ref().is_some_and(|conversation| {
                state
                    .tasks
                    .iter()
                    .any(|task| task.status.conversation == *conversation)
            }) {
                return;
            }
            if state.warm.as_ref().is_some_and(|warm| {
                warm.lane == lane
                    && warm.conversation == conversation
                    && warm.window == window
                    && now.saturating_sub(warm.at_ms) < PREWARM_FOR_MS
            }) {
                return;
            }
            let session = match &state.session {
                Session::On(id) => Some(id.clone()),
                _ => None,
            };
            // A Background task joins a session that starts with it; its
            // arguments can't be known ahead.
            if lane == Lane::Background && settings.control_desktop && session.is_none() {
                return;
            }
            let key = self.next.fetch_add(1, Ordering::Relaxed);
            let id = conversation.clone().unwrap_or_else(store::new_id);
            state.warm = Some(Warm {
                lane,
                conversation: conversation.clone(),
                id: id.clone(),
                key,
                window: window.clone(),
                at_ms: now,
            });
            (conversation, id, key, session)
        };
        let job = Job {
            conversation: id,
            request: Request {
                conversation,
                text: String::new(),
                origin: match lane {
                    Lane::Here => Origin::ActionShortcut,
                    Lane::Background => Origin::Background,
                },
                context: Context {
                    window,
                    ..Context::default()
                },
            },
            agent_id: format!("hex-{key}"),
            desktop_session: session.filter(|_| settings.control_desktop),
            settings,
            kind: JobKind::Ask,
            digest: None,
        };
        let runner = Arc::clone(self);
        std::thread::Builder::new()
            .name("assistant-prewarm".into())
            .spawn(move || (runner.hooks.prewarm)(&job))
            .ok();
    }

    pub(crate) fn resume_interrupted(self: &Arc<Self>, conversation: &str) -> Option<ConversationId> {
        let (origin, question) = (self.hooks.interrupted)(conversation)?;
        let origin = match origin {
            Origin::ActionShortcut => Origin::Background,
            other => other,
        };
        Some(self.submit_as(
            Request {
                conversation: Some(conversation.to_owned()),
                text: question,
                origin,
                context: Context::default(),
            },
            JobKind::Resume,
        ))
    }

    pub(crate) fn cancel(self: &Arc<Self>, conversation: &str) {
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

    pub(crate) fn cancel_all(self: &Arc<Self>) {
        let keys: Vec<u64> = self.state().tasks.iter().map(|task| task.key).collect();
        for key in keys {
            self.cancel_task(key);
        }
        self.voice.stop();
        (self.hooks.stop_speaking)();
    }

    /// Stops a running task (it ends with `Cancelled` on its own thread), or
    /// drops a queued one and says so.
    fn cancel_task(self: &Arc<Self>, key: u64) {
        let (dropped, here, alone) = {
            let mut state = self.state();
            let Some(index) = state.tasks.iter().position(|task| task.key == key) else {
                return;
            };
            let task = &state.tasks[index];
            let here = task.lane() == Lane::Here;
            task.control.cancel();
            if task.running {
                (None, here, false)
            } else {
                let task = state.tasks.remove(index);
                let alone = !state
                    .tasks
                    .iter()
                    .any(|other| other.status.conversation == task.status.conversation);
                (Some(task.status.conversation), here, alone)
            }
        };
        if here {
            self.voice.stop();
            (self.hooks.stop_speaking)();
        }
        if let Some(conversation) = dropped {
            if alone {
                (self.hooks.release)(&conversation);
                (self.hooks.settle)(&conversation, Ended::Cancelled, "Stopped.");
            }
            self.emit(&Event::Cancelled { conversation });
            self.refresh_mode();
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
        let now = self.now();
        focus_in(&self.state(), follow_up_seconds, now)
    }
}

/// `focus` on `state`: a running Here task, Chats' conversation, or the last
/// Here answer within `follow_up_seconds`.
fn focus_in(state: &State, follow_up_seconds: u32, now: u64) -> Option<ConversationId> {
    if let Some(task) = state
        .tasks
        .iter()
        .find(|task| task.lane() == Lane::Here && !task.control.cancelled())
    {
        return Some(task.status.conversation.clone());
    }
    if let Some(focus) = &state.focus {
        return Some(focus.clone());
    }
    let (id, answered) = state.last_quick.as_ref()?;
    (follow_up_seconds > 0 && now.saturating_sub(*answered) <= u64::from(follow_up_seconds) * 1000)
        .then(|| id.clone())
}

fn running_task<'a>(state: &'a mut State, conversation: &str) -> Option<&'a mut Task> {
    state
        .tasks
        .iter_mut()
        .find(|task| task.running && task.status.conversation == conversation)
}

/// How long to wait after the `attempt`th limit that named no reset: one
/// minute, doubling, at most half an hour.
fn backoff(attempt: u32) -> u64 {
    (60_000u64 << attempt.saturating_sub(1).min(5)).min(30 * 60_000)
}

/// The agent-desktop session around Background work: one multi-agent cursor
/// session, every task its own agent (and cursor) in it.
mod desktop_session {
    use std::process::Stdio;

    use serde_json::Value;

    use crate::assistant::types::TaskWindow;

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
            "Hex background tasks",
        ]);
        let id = reply
            .as_ref()
            .and_then(|reply| reply.pointer("/data/session_id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if id.is_none() {
            tracing::warn!(?reply, "agent-desktop didn't start a session for background tasks");
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

    /// Brings the window `task` holds (its lease, else the last window it
    /// named) forward: the person asked to take it over, so this one
    /// physical change is theirs.
    pub(super) fn bring_forward(task: &str, windows: &[TaskWindow]) {
        let leased: Vec<TaskWindow> = crate::assistant::guard::leases()
            .into_iter()
            .filter(|lease| lease.task == task)
            .map(|lease| lease.window)
            .collect();
        let window = leased
            .iter()
            .chain(windows.iter().rev())
            .find(|window| window.window_id.is_some())
            .or_else(|| leased.first().or(windows.last()))
            .cloned();
        let Some(window) = window else {
            return;
        };
        std::thread::spawn(move || {
            let reply = match &window.window_id {
                Some(id) => agent_desktop(&["focus-window", "--window-id", id]),
                None => agent_desktop(&["focus-window", "--app", &window.app]),
            };
            if reply.and_then(|reply| reply.get("ok").and_then(Value::as_bool)) != Some(true) {
                tracing::warn!(app = window.app, "couldn't bring the task's window forward");
            }
        });
    }
}

#[cfg(test)]
mod tests;
