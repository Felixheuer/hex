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
mod inbox;
mod monitor;
pub mod remote;
mod run;
mod session;
mod status;
mod voice;
mod warm;

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::app_settings::{AssistantBackend, AssistantSettings};

use super::agent::{self, Job, JobKind, Outcome};
use super::autonomy::Authorized;
use super::notify::Note;
use super::provider::{self, Control};
use super::schedules::Schedule;
use super::store::{self, Store};
use super::types::{
    Context, Conversation, ConversationId, ConversationSummary, Event, Lane, Origin,
    Outcome as Ended, PlanStep, Request, TaskState, TaskStatus, TaskWindow,
};

/// Called on a worker thread for every event; keep it quick (post to your
/// own thread or wake your loop).
pub type Subscriber = Box<dyn Fn(&Event) + Send + Sync>;

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

/// The questions tasks wait on: the one the next hold answers, and the
/// others behind it (`Event::QuestionsChanged` tells when it changes).
pub fn inbox() -> Inbox {
    runner().inbox()
}

/// The person is holding the Action shortcut or dictating: questions and
/// finished jobs wait to be read aloud until they stop (or 30 s pass).
pub fn set_listening(on: bool) {
    voice::set_listening(on, store::now_ms());
}

/// Whether questions and finished jobs are read aloud (the default), or
/// only shown, for a surface that plays a soft chime instead.
#[allow(dead_code, reason = "for the overlay's chime setting")]
pub fn set_spoken_announcements(on: bool) {
    voice::set_spoken_announcements(on);
}

/// Whether an answer is still being read aloud or is about to be: its
/// sentences wait their turn, or more of it is still streaming. Together
/// with `speech::speaking()` (what the engine plays or makes now), it tells
/// conversation mode the reading isn't over between two sentences.
pub fn reading_aloud() -> bool {
    runner().voice.reading()
}

/// Whether questions and finished jobs are read aloud, for the overlay
/// to chime when they aren't.
pub fn spoken_announcements() -> bool {
    voice::spoken_announcements()
}

/// Puts `conversation`'s question behind the others (the overlay's
/// Later; saying "später" does the same), and brings up the next.
pub fn defer_question(conversation: &str) {
    runner().defer(conversation);
}

pub use inbox::Inbox;

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

/// The plan of a running or finished job (its `plan.md` checklist), if it
/// made one.
pub fn plan(conversation: &str) -> Vec<PlanStep> {
    runner().plan(conversation)
}

/// The question a task is waiting on (`TaskState::NeedsInput`), if any.
pub fn question(conversation: &str) -> Option<String> {
    runner().question(conversation)
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
    super::autonomy::forget(id);
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

/// The conversation the next Here request should continue: a task waiting
/// for the person's answer (`NeedsInput`; the request is the answer), else a
/// Here task running now (it is steered), else the one Chats shows, else the
/// last Here conversation answered within `follow_up_seconds`, else `None`
/// (start a new one).
pub fn focus(follow_up_seconds: u32) -> Option<ConversationId> {
    runner().focus(follow_up_seconds)
}

/// Starts the scheduler (once): every half minute it starts the scheduled
/// jobs that are due, as Background jobs; a run missed while the Mac slept
/// or Hex was closed starts once. The app calls it at launch (the runner
/// also starts it when it first runs in the app).
pub fn start_scheduler() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::Builder::new()
        .name("assistant-scheduler".into())
        .spawn(|| {
            loop {
                if current_settings().0 {
                    runner().run_due_schedules();
                }
                std::thread::sleep(Duration::from_secs(30));
            }
        })
        .ok();
}

/// Starts the scheduled jobs due now (`hex assistant schedules watch`);
/// their conversations.
pub fn run_due_schedules() -> Vec<ConversationId> {
    runner().run_due_schedules()
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
        // assistant` command must not clear the app's pauses, or run its
        // schedules).
        std::thread::spawn(|| {
            if !WITHOUT_LAB.load(Ordering::Acquire) && current_settings().0 {
                super::guard::startup();
                start_scheduler();
                // The Chrome logins Hex's browsers use, taken again daily.
                let profile = current_settings().1.browser_import_profile;
                std::thread::spawn(move || {
                    super::browser::accounts::refresh_if_stale(profile.as_deref());
                });
            }
            let app = !WITHOUT_LAB.load(Ordering::Acquire) && current_settings().0;
            if let Ok(store) = Store::open() {
                let recovered = store.recover_interrupted();
                if !recovered.is_empty() {
                    tracing::info!(count = recovered.len(), "assistant tasks were interrupted");
                }
                // Jobs go on by themselves: a restart, a crash, or an
                // update doesn't end days of work.
                if app {
                    let budget = u64::from(current_settings().1.job_budget_minutes) * 60_000;
                    for id in store.resumable_jobs(&recovered, store::now_ms(), budget) {
                        tracing::info!(conversation = id, "resuming an interrupted job");
                        let _ = runner().resume_interrupted(&id);
                    }
                }
            }
        });
        Runner::new(Hooks::system())
    })
}

type Execute = dyn Fn(&Job, &Control, &mut dyn FnMut(Event)) -> Outcome + Send + Sync;
type StyleAgent = dyn Fn(&str, &str, u8, Option<&str>) + Send + Sync;
type Speak = dyn Fn(&str, Option<&str>) + Send + Sync;
type Hook<T> = Box<dyn Fn() -> T + Send + Sync>;
type OnTask<R = ()> = Box<dyn Fn(&str) -> R + Send + Sync>;
type TaskHook = OnTask;
type Pause = dyn Fn(&str, bool) + Send + Sync;
type BringForward = dyn Fn(&str, &[TaskWindow]) + Send + Sync;
type Settle = dyn Fn(&str, Ended, &str) + Send + Sync;
type Authorize = dyn Fn(&str, &Authorized) + Send + Sync;
type DueSchedules = dyn Fn(u64) -> Vec<Schedule> + Send + Sync;
/// (question, task title, what the person said) → whether it answers the
/// question; `None` when it can't tell in time.
type ClassifyReply = dyn Fn(&str, &str, &str) -> Option<bool> + Send + Sync;

/// What the runner does in the world; tests replace each part.
pub(crate) struct Hooks {
    pub execute: Box<Execute>,
    /// Whether the assistant is on, and its settings, now.
    pub settings: Hook<(bool, AssistantSettings)>,
    /// Starts an agent-desktop multi-agent cursor session; its id.
    pub start_session: Hook<Option<String>>,
    pub end_session: TaskHook,
    /// Gives agent `agent_id` in `session` its colour, and a label with its
    /// current step.
    pub style_agent: Box<StyleAgent>,
    /// Starts reading text aloud with a voice (returns at once).
    pub speak: Box<Speak>,
    /// Whether something is being read aloud.
    pub speaking: Hook<bool>,
    /// The voice for an answer, from its first sentence, when none is set.
    pub pick_voice: OnTask<Option<String>>,
    pub stop_speaking: Hook<()>,
    /// Unix ms now, and a pause: the runner's clock.
    pub now_ms: Hook<u64>,
    pub sleep: Box<dyn Fn(Duration) + Send + Sync>,
    /// Pauses or resumes a task's desktop actions (`guard::set_paused`).
    pub pause: Box<Pause>,
    /// A task ended: its window leases and browser go (`guard::release_all`,
    /// `browser::close`).
    pub release: TaskHook,
    /// The task moving the real pointer now (`guard::hands_off`).
    pub hands_off: Hook<Option<ConversationId>>,
    /// Brings a task's window forward (taken over).
    pub bring_forward: Box<BringForward>,
    /// Records how a conversation's request ended when its run didn't.
    pub settle: Box<Settle>,
    /// The latest stored conversations, newest first.
    pub recent: Hook<Vec<ConversationSummary>>,
    /// An interrupted conversation's lane and last question.
    pub interrupted: OnTask<Option<(Origin, String)>>,
    /// Starts a job's backend ahead of its question.
    pub prewarm: Box<dyn Fn(&Job) + Send + Sync>,
    /// Requests from other Hex processes (`remote`).
    pub requests: Hook<Vec<(String, remote::Verb)>>,
    /// Makes a new job's workspace for its title.
    pub make_workspace: OnTask<Option<std::path::PathBuf>>,
    /// A conversation's workspace, if it is a job.
    pub workspace_of: OnTask<Option<std::path::PathBuf>>,
    /// Reads and writes what the person authorized for a task (`autonomy`).
    pub authorized: OnTask<Authorized>,
    pub authorize: Box<Authorize>,
    /// The person's memory note.
    pub memory: Hook<String>,
    /// Shows a job's notification.
    pub notify: Box<dyn Fn(&Note) + Send + Sync>,
    /// The schedules due at a time, marked as run (`schedules::take_due`).
    pub due_schedules: Box<DueSchedules>,
    /// Settles an utterance the inbox's router can't place (`inbox`).
    pub classify_reply: Box<ClassifyReply>,
}

impl Hooks {
    /// The real ones: the backends, the saved settings, agent-desktop, the
    /// guard, the browser, the store, and the macOS voice.
    fn system() -> Self {
        let seen = Mutex::new(std::collections::HashSet::new());
        Self {
            execute: Box::new(agent::run),
            settings: Box::new(current_settings),
            start_session: Box::new(session::desktop_session::start),
            end_session: Box::new(session::desktop_session::end),
            style_agent: Box::new(session::desktop_session::style),
            speak: Box::new(speak_next),
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
            bring_forward: Box::new(session::desktop_session::bring_forward),
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
            make_workspace: Box::new(|title| {
                let root = agent::workspace::root()?;
                agent::workspace::create(&root, title, store::now_ms())
                    .inspect_err(|error| tracing::warn!(%error, "no workspace for the job"))
                    .ok()
            }),
            workspace_of: Box::new(|conversation| {
                Store::open().ok()?.load(conversation)?.workspace
            }),
            authorized: Box::new(super::autonomy::read),
            authorize: Box::new(super::autonomy::write),
            memory: Box::new(super::memory::read),
            notify: Box::new(super::notify::show),
            due_schedules: Box::new(super::schedules::take_due),
            classify_reply: Box::new(|question, title, said| {
                let (_, settings) = current_settings();
                (settings.backend == AssistantBackend::ClaudeApi).then_some(())?;
                let model = settings
                    .here_model
                    .clone()
                    .unwrap_or_else(|| settings.claude_api_model.clone());
                provider::classify_reply(&model, question, title, said, inbox::CLASSIFY_FOR)
            }),
        }
    }
}

/// The settings as applied, or as saved (read at most every two seconds)
/// for a `hex` command that never applies them.
/// Reads `text` right after what is being read, with no gap between
/// sentences, or starts reading it when nothing is.
fn speak_next(text: &str, voice: Option<&str>) {
    if super::speech::speaking() {
        super::speech::queue(text, voice);
    } else {
        super::speech::speak(text, voice);
    }
}

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
    /// The conversation the person last spoke to: its question is answered
    /// first (`inbox`).
    last_addressed: Option<ConversationId>,
    /// The inbox as last told (`Event::QuestionsChanged`): its active
    /// conversation and how many wait.
    told_inbox: Option<(ConversationId, usize)>,
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
    /// Its time limit in minutes, for the message when it runs out.
    limit_minutes: u32,
    /// When it was queued (runner clock), for the status told on request.
    started_ms: u64,
    /// Since when it has been paused or waiting (not counted to its time).
    held_since: Option<u64>,
    /// Its state before it was taken over.
    before_pause: TaskState,
    /// It was asked by voice: its question and a job's answer are spoken.
    voice: bool,
    /// A job's folder, where its plan is read from.
    workspace: Option<std::path::PathBuf>,
    /// The desktop session its cursor is in, and the cursor's label.
    session: Option<String>,
    label: Option<String>,
    /// A job's plan, and when its file last changed.
    plan: Vec<PlanStep>,
    plan_seen: Option<std::time::SystemTime>,
    /// The question it waits on (`NeedsInput`), and the answer once given.
    question: Option<String>,
    /// When it asked (runner clock); its place in the inbox.
    asked_ms: Option<u64>,
    reply: Option<String>,
    /// The plan's `HEX_AUTHORIZED:` line last applied.
    authorized_line: Option<String>,
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
                let mut relabel = None;
                if let Some(task) = running_task(&mut self.state(), conversation) {
                    task.status.step = Some(description.clone());
                    task.steps.push_back(description.clone());
                    while task.steps.len() > STEPS_KEPT {
                        task.steps.pop_front();
                    }
                    let label = cursor_label(description);
                    if let Some(session) = &task.session
                        && task.label.as_deref() != Some(label.as_str())
                    {
                        task.label = Some(label.clone());
                        relabel = Some((
                            session.clone(),
                            task.agent_id.clone(),
                            task.status.colour,
                            label,
                        ));
                    }
                }
                if let Some((session, agent, colour, label)) = relabel {
                    let runner = Arc::clone(self);
                    std::thread::spawn(move || {
                        (runner.hooks.style_agent)(&session, &agent, colour, Some(&label));
                    });
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
        if !settings.setup_done && !super::trying() {
            return refuse("Finish setting up the assistant first (Settings › Assistant).");
        }
        // "Wie weit ist der Shop?": told from the jobs' plans, never asked
        // of an agent; jobs don't announce their steps on their own.
        if kind == JobKind::Ask
            && let Some(told) = self.status_reply(&request)
        {
            return told;
        }
        if kind == JobKind::Ask
            && let Some(handled) = self.route_hold(&mut request)
        {
            return handled;
        }
        let now = self.now();
        let voice = request.origin != Origin::Chat;
        let mut stop = Vec::new();
        let mut on_it = None;
        let id = {
            let mut state = self.state();
            // The answer to a job's question.
            if kind == JobKind::Ask
                && let Some(conversation) = &request.conversation
                && let Some(task) = state.tasks.iter_mut().find(|task| {
                    task.status.conversation == *conversation
                        && task.status.state == TaskState::NeedsInput
                })
            {
                task.reply = Some(request.text.trim().to_owned());
                state.last_addressed = Some(conversation.clone());
                return conversation.clone();
            }
            if kind == JobKind::Ask {
                let running: Vec<TaskStatus> = state
                    .tasks
                    .iter()
                    .filter(|task| task.running)
                    .map(|task| task.status.clone())
                    .collect();
                // Words that name a running task ("bei der Shop-Aufgabe …")
                // go to it, also from a follow-up of another conversation
                // (said over an answer, or hands-free after it).
                if let Some(named) = context::route(&request.text, &running) {
                    request.conversation = Some(named);
                }
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
            // A new Background request is a job; a new Here request that is
            // plainly one goes straight to the background (others are
            // routed by the Here agent itself, `agent::ROUTING`).
            if kind == JobKind::Ask && new_conversation {
                let plainly = request.origin == Origin::ActionShortcut
                    && context::looks_like_job(&request.text);
                if plainly {
                    request.origin = Origin::Background;
                    request.context.window = None;
                    on_it = Some(context::on_it(&request.text));
                }
                if request.origin == Origin::Background {
                    kind = JobKind::Job {
                        title: store::shorten(&request.text, 48),
                    };
                }
            }
            if request.origin.lane() == Lane::Here {
                stop = state
                    .tasks
                    .iter()
                    .filter(|task| task.lane() == Lane::Here && task.status.conversation != id)
                    .map(|task| task.key)
                    .collect();
            }
            request.conversation = Some(id.clone());
            self.push_task(&mut state, request, kind, key, new_conversation, voice);
            id
        };
        for key in stop {
            self.cancel_task(key);
        }
        if let Some(line) = on_it {
            self.emit(&Event::Escalated {
                conversation: id.clone(),
            });
            if settings.speak_answers {
                self.voice.finish(self, &id, &line);
            }
        }
        self.refresh_mode();
        self.pump();
        id
    }

    /// Adds a task for `request` (whose conversation is set) to the queue.
    fn push_task(
        &self,
        state: &mut State,
        request: Request,
        kind: JobKind,
        key: Option<u64>,
        new_conversation: bool,
        voice: bool,
    ) {
        let id = request.conversation.clone().unwrap_or_default();
        let key = key.unwrap_or_else(|| self.next.fetch_add(1, Ordering::Relaxed));
        let used: Vec<u8> = state.tasks.iter().map(|task| task.status.colour).collect();
        let colour = (0..8u8)
            .find(|colour| !used.contains(colour))
            .unwrap_or((key % 8) as u8);
        let started_ms = self.now();
        state.tasks.push(Task {
            key,
            status: TaskStatus {
                conversation: id,
                origin: request.origin,
                title: store::shorten(&request.text, 60),
                state: TaskState::Queued,
                step: None,
                windows: Vec::new(),
                colour,
            },
            request,
            control: Control::new(),
            agent_id: format!("hex-{key}"),
            running: false,
            announced: false,
            kind,
            new_conversation,
            speak: false,
            voice,
            steps: VecDeque::new(),
            deadline_ms: None,
            limit_minutes: 0,
            started_ms,
            held_since: None,
            before_pause: TaskState::Running,
            workspace: None,
            session: None,
            label: None,
            plan: Vec::new(),
            plan_seen: None,
            question: None,
            asked_ms: None,
            reply: None,
            authorized_line: None,
        });
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

    pub(crate) fn resume_interrupted(
        self: &Arc<Self>,
        conversation: &str,
    ) -> Option<ConversationId> {
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

    /// Starts each due schedule as a Background job; their conversations.
    pub(crate) fn run_due_schedules(self: &Arc<Self>) -> Vec<ConversationId> {
        let now = self.now();
        (self.hooks.due_schedules)(now)
            .into_iter()
            .map(|schedule| {
                tracing::info!(id = schedule.id, "a scheduled job is due");
                self.submit(Request {
                    conversation: None,
                    text: schedule.task,
                    origin: Origin::Background,
                    context: Context::default(),
                })
            })
            .collect()
    }

    pub(crate) fn question(&self, conversation: &str) -> Option<String> {
        self.state()
            .tasks
            .iter()
            .find(|task| task.status.conversation == conversation)
            .and_then(|task| task.question.clone())
    }

    pub(crate) fn plan(&self, conversation: &str) -> Vec<PlanStep> {
        let (plan, workspace) = {
            let state = self.state();
            match state
                .tasks
                .iter()
                .find(|task| task.status.conversation == conversation)
            {
                Some(task) => (task.plan.clone(), task.workspace.clone()),
                None => (Vec::new(), None),
            }
        };
        if !plan.is_empty() {
            return plan;
        }
        workspace
            .or_else(|| (self.hooks.workspace_of)(conversation))
            .map(|workspace| agent::workspace::plan(&workspace))
            .unwrap_or_default()
    }

    pub(crate) fn focus(&self, follow_up_seconds: u32) -> Option<ConversationId> {
        let now = self.now();
        focus_in(&self.state(), follow_up_seconds, now)
    }
}

/// `focus` on `state`: the inbox's active question, a running Here task,
/// Chats' conversation, or the last Here answer within `follow_up_seconds`.
fn focus_in(state: &State, follow_up_seconds: u32, now: u64) -> Option<ConversationId> {
    if let Some(active) = inbox::of(state).active {
        return Some(active.conversation);
    }
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

/// A step as the cursor's label: at most six words.
fn cursor_label(step: &str) -> String {
    step.split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
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

#[cfg(test)]
mod tests;
