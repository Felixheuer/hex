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

mod focus;
use focus::{focus_in, focus_past, job_named};
mod context;
mod inbox;
mod monitor;
pub mod remote;
mod run;
mod session;
mod status;
mod team;
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

/// Whose words a task's authorization (`autonomy::from_request`) may come
/// from: only the person's own, never text an agent wrote (a hand-off's
/// task, a team member's, a schedule an agent made), which a page or an
/// email it read could have steered.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Said {
    /// The request's text is the person's own words.
    Request,
    /// The person's words that led to this (agent-written) task.
    Words(String),
    /// Nothing the person said now: only what was authorized before.
    Nobody,
}

/// Called on a worker thread for every event; keep it quick (post to your
/// own thread or wake your loop).
pub type Subscriber = Box<dyn Fn(&Event) + Send + Sync>;

/// How long a Background task waits for the desktop session to start.
const SESSION_WAIT: Duration = Duration::from_secs(8);
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

/// Passes `text` to the running job `to` names (its short id from the
/// stream's digest, or words of its title) as the person's instruction;
/// whether one was found (`hex_tell_job`).
pub fn tell_job(to: &str, text: &str) -> bool {
    runner().tell_job(to, text)
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

/// The words of the Action shortcut's hold so far (the live transcript, as
/// it streams): what they will likely need starts now, before the release
/// (the reserved quick conversation's browser when they ask for the web).
/// Cheap and quiet to call on every change; does nothing outside a hold.
pub fn speculate(partial: &str) {
    if partial.trim().is_empty()
        || super::interaction::listening() != super::interaction::Listening::Hold
    {
        return;
    }
    runner().speculate(partial);
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

/// Hex goes quiet at once: the line being read stops and the rest of that
/// answer is dropped; questions and summaries waiting their turn stay
/// (`interaction::hush`).
pub fn hush() {
    let runner = runner();
    runner.voice.stop();
    (runner.hooks.stop_speaking)();
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

/// The screen frame's state (Agent mode is gone: the frame follows the
/// work, `frame_shown`).
pub fn agent_mode() -> bool {
    frame_shown()
}

/// Whether the screen frame shows: any task is working (a Here question
/// thinking or acting, or Background work), not just waiting for the
/// person's answer (`session::frame_visible`).
pub fn frame_shown() -> bool {
    runner().state().frame_on
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

/// `focus` without the inbox's active question: for a hold during which
/// that question came up, unheard.
pub fn focus_past_inbox(follow_up_seconds: u32) -> Option<ConversationId> {
    let runner = runner();
    let now = runner.now();
    focus_past(&runner.state(), follow_up_seconds, now)
}

/// Jobs' summaries and questions aren't read aloud for `quiet` (the person
/// said „Stopp“ or pressed Escape); what they ask for still is.
pub fn quiet_announcements(quiet: std::time::Duration) {
    voice::quiet_for(quiet, store::now_ms());
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
    // Replies from the person's phone while they're away.
    crate::assistant::phone::start();
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
pub fn run_from_cli() {
    FROM_CLI.store(true, Ordering::Release);
}

static FROM_CLI: AtomicBool = AtomicBool::new(false);

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
            if !FROM_CLI.load(Ordering::Acquire) && current_settings().0 {
                super::guard::startup();
                start_scheduler();
                // The Chrome logins Hex's browsers use, taken again daily.
                let profile = current_settings().1.browser_import_profile;
                std::thread::spawn(move || {
                    super::browser::accounts::refresh_if_stale(profile.as_deref());
                });
            }
            let app = !FROM_CLI.load(Ordering::Acquire) && current_settings().0;
            // Agents an earlier Hex left running (a crash, a force quit)
            // end before their jobs resume beside them.
            if app {
                let ended = super::provider::orphans::sweep();
                if ended > 0 {
                    tracing::info!(ended, "ended agents an earlier Hex left running");
                }
            }
            if let Ok(store) = Store::open() {
                let recovered = store.recover_interrupted();
                if !recovered.is_empty() {
                    tracing::info!(count = recovered.len(), "assistant tasks were interrupted");
                }
                // Their browsers go too, all at once and before any job
                // resumes (it opens fresh ones under the same names).
                if app {
                    let closing: Vec<_> = recovered
                        .iter()
                        .cloned()
                        .map(|id| std::thread::spawn(move || super::browser::close(&id)))
                        .collect();
                    for closed in closing {
                        let _ = closed.join();
                    }
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
/// Called with a task and its request's words.
type OnWords = dyn Fn(&str, &str) + Send + Sync;
type OnTask<R = ()> = Box<dyn Fn(&str) -> R + Send + Sync>;
type TaskHook = OnTask;
/// Makes a job's workspace for its title, inside a folder (a helper's, in
/// its lead's) or in the jobs' own root.
type MakeWorkspace =
    dyn Fn(Option<&std::path::Path>, &str) -> Option<std::path::PathBuf> + Send + Sync;
type Pause = dyn Fn(&str, bool) + Send + Sync;
type BringForward = dyn Fn(&str, &[TaskWindow]) + Send + Sync;
type Settle = dyn Fn(&str, Ended, &str) + Send + Sync;
type Authorize = dyn Fn(&str, &Authorized) + Send + Sync;
type DueSchedules = dyn Fn(u64) -> Vec<Schedule> + Send + Sync;
type WriteNote = dyn Fn(&str, super::types::Message) + Send + Sync;
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
    /// The window in front now (`computer::quick_context`), for a backend
    /// kept ready after a quick answer (`stand_by`).
    pub front_window: Hook<Option<TaskWindow>>,
    /// Starts a task's browser ahead for its request's words
    /// (`browser::prewarm_for`).
    pub browser_ahead: Box<OnWords>,
    /// Requests from other Hex processes (`remote`).
    pub requests: Hook<Vec<(String, remote::Verb)>>,
    /// Makes a new job's workspace for its title.
    pub make_workspace: Box<MakeWorkspace>,
    /// A conversation's workspace, if it is a job.
    pub workspace_of: OnTask<Option<std::path::PathBuf>>,
    /// Writes a message into a conversation beside the task that may be
    /// running it (`Store::append`): a status answer, a note said while a
    /// job works, so it sits where it was said.
    pub note: Box<WriteNote>,
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
    /// How tight memory is now: queued Background tasks wait while it is
    /// (`capacity::background_room`).
    pub pressure: Hook<super::capacity::Pressure>,
    /// Jobs' requests to start helpers (`agent::team`), taken once.
    pub team_asks: Hook<Vec<agent::team::Ask>>,
    /// Answers a request: the helpers started, or why not.
    pub team_answer: Box<TeamAnswer>,
    /// Every event, for the helpers' records.
    pub team_event: Box<dyn Fn(&Event) + Send + Sync>,
    /// The lead a conversation helps, if it's a helper (after a restart).
    pub lead_of: OnTask<Option<ConversationId>>,
}

type TeamAnswer = dyn Fn(&agent::team::Ask, Result<Vec<agent::team::Member>, String>) + Send + Sync;

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
            front_window: Box::new(|| super::computer::quick_context().window),
            browser_ahead: Box::new(super::browser::prewarm_for),
            requests: Box::new(move || {
                remote::take_new(&mut seen.lock().unwrap_or_else(|error| error.into_inner()))
            }),
            make_workspace: Box::new(|inside, title| {
                let root = match inside {
                    Some(folder) => folder.to_path_buf(),
                    None => agent::workspace::root()?,
                };
                agent::workspace::create(&root, title, store::now_ms())
                    .inspect_err(|error| tracing::warn!(%error, "no workspace for the job"))
                    .ok()
            }),
            workspace_of: Box::new(|conversation| {
                Store::open().ok()?.load(conversation)?.workspace
            }),
            note: Box::new(|conversation, message| {
                if let Err(error) =
                    Store::open().and_then(|store| store.append(conversation, message))
                {
                    tracing::warn!(%error, "couldn't add a note to the conversation");
                }
            }),
            authorized: Box::new(super::autonomy::read),
            authorize: Box::new(super::autonomy::write),
            memory: Box::new(super::memory::read),
            notify: Box::new(super::notify::show),
            due_schedules: Box::new(super::schedules::take_due),
            pressure: Box::new(super::capacity::pressure),
            team_asks: Box::new(|| {
                agent::team::root()
                    .map(|root| agent::team::pending(&root))
                    .unwrap_or_default()
            }),
            team_answer: Box::new(|ask, result| {
                if let Some(root) = agent::team::root() {
                    agent::team::answer(&root, ask, result);
                }
            }),
            team_event: Box::new(|event| {
                if let Some(root) = agent::team::root() {
                    agent::team::on_event(&root, event);
                }
            }),
            lead_of: Box::new(agent::team::lead_of),
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
    let from_cli = FROM_CLI.load(Ordering::Acquire);
    if from_cli {
        settings.setup_done = true;
    }
    (on || from_cli, settings)
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
    /// Background work is queued, running, waiting, or paused (the desktop
    /// session runs around it; `background_busy`).
    background_on: bool,
    /// The screen frame shows (`frame_shown`): any task working.
    frame_on: bool,
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
    /// The reserved conversation whose browser a hold's words already
    /// started (`speculate`).
    speculated: Option<ConversationId>,
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
    /// How long it is kept for its request (`agent::ahead_for`).
    for_ms: u64,
}

struct Task {
    key: u64,
    request: Request,
    /// The person's own words this task may be authorized from (`Said`).
    said: Option<String>,
    /// The job it helps (`agent::team`): it works in a folder inside that
    /// job's, stays quiet (the lead answers), and stops with it.
    parent: Option<ConversationId>,
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
        (self.hooks.team_event)(event);
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

    /// [`tell_job`]: steers the one running task `to` names.
    pub(crate) fn tell_job(self: &Arc<Self>, to: &str, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let found = {
            let state = self.state();
            let running: Vec<TaskStatus> = state
                .tasks
                .iter()
                .filter(|task| !task.control.cancelled())
                .map(|task| task.status.clone())
                .collect();
            job_named(to, &running)
        };
        let Some(conversation) = found else {
            tracing::info!(to, "hex_tell_job named no running job");
            return false;
        };
        self.submit(Request {
            conversation: Some(conversation),
            text: text.to_owned(),
            origin: Origin::Chat,
            context: Context::default(),
        });
        true
    }

    pub(crate) fn submit(self: &Arc<Self>, request: Request) -> ConversationId {
        self.submit_as(request, JobKind::Ask, Said::Request)
    }

    fn submit_as(
        self: &Arc<Self>,
        mut request: Request,
        mut kind: JobKind,
        said: Said,
    ) -> ConversationId {
        // „Ich gehe jetzt zum Friseur …“: from now on the phone hears it.
        crate::assistant::phone::note_request(&request.text);
        let (enabled, settings) = (self.hooks.settings)();
        // Started first, so the Here bubble has the question to say why on.
        let refuse = |message: &str| {
            let id = request.conversation.clone().unwrap_or_else(store::new_id);
            self.emit(&Event::Started {
                conversation: id.clone(),
                origin: request.origin,
                question: request.text.clone(),
            });
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
            return refuse(&format!(
                "Finish setting up the assistant first ({}).",
                super::lines::ASSISTANT_SETTINGS
            ));
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
        // In a job's own conversation: "wie weit bist du?" is told at once,
        // and words said while it waits for its turn join its request.
        if kind == JobKind::Ask
            && let Some(handled) = self.in_job(&request)
        {
            return handled;
        }
        let now = self.now();
        let voice = request.origin != Origin::Chat;
        let mut stop = Vec::new();
        let mut on_it = None;
        let mut steer_note = None;
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
            let mut steered_parent: Option<ConversationId> = None;
            let mut steered_title: Option<String> = None;
            if let Some(warm) = state.warm.take()
                && kind == JobKind::Ask
                && warm.lane == request.origin.lane()
                && warm.conversation == request.conversation
                && warm.window == request.context.window
                && now.saturating_sub(warm.at_ms) < warm.for_ms
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
                // A helper steered stays its lead's, and a job keeps its
                // title (the new words are an instruction, not a name).
                steered_parent = target.parent.clone();
                if target.workspace.is_some() || matches!(target.kind, JobKind::Job { .. }) {
                    steered_title = Some(target.status.title.clone());
                }
                // A job's transcript gets the words where they were said,
                // with a short "noted"; its steered run doesn't add them again.
                if target.workspace.is_some() || matches!(target.kind, JobKind::Job { .. }) {
                    steer_note = Some((id.clone(), request.text.clone()));
                }
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
            let said = match said {
                Said::Request => Some(request.text.clone()),
                Said::Words(words) => Some(words),
                Said::Nobody => None,
            };
            self.push_task(
                &mut state,
                request,
                kind,
                key,
                new_conversation,
                voice,
                said,
            );
            if let Some(task) = state.tasks.last_mut() {
                if steered_parent.is_some() {
                    task.parent = steered_parent;
                }
                if let Some(title) = steered_title {
                    task.status.title = title;
                }
            }
            id
        };
        for key in stop {
            self.cancel_task(key);
        }
        if let Some((conversation, said)) = steer_note {
            self.note(&conversation, super::types::Role::User, &said);
            self.note(
                &conversation,
                super::types::Role::Assistant,
                super::lines::tr("Okay, I'll take that into account at the next step.", &said),
            );
            self.emit(&Event::ConversationsChanged);
        }
        if let Some(line) = on_it {
            // Shown where it's heard (the Here bubble), then the job goes on.
            self.emit(&Event::AnswerDelta {
                conversation: id.clone(),
                text: line.clone(),
            });
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
    #[allow(clippy::too_many_arguments)]
    fn push_task(
        &self,
        state: &mut State,
        request: Request,
        kind: JobKind,
        key: Option<u64>,
        new_conversation: bool,
        voice: bool,
        said: Option<String>,
    ) {
        let id = request.conversation.clone().unwrap_or_default();
        let id_for_parent = id.clone();
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
            said,
            parent: (self.hooks.lead_of)(&id_for_parent),
        });
    }

    /// Starts every queued task that has a free slot in its lane and whose
    /// conversation isn't already running; announces the rest as queued.
    fn pump(self: &Arc<Self>) {
        let (_, settings) = (self.hooks.settings)();
        let limit = settings.max_parallel.max(1) as usize;
        let pressure = (self.hooks.pressure)();
        let mut start = Vec::new();
        let mut queued = Vec::new();
        let mut monitor = false;
        {
            let mut state = self.state();
            // A job whose helpers are queued or working mostly waits for
            // them: it holds no slot, or leads holding every slot would
            // wait for helpers that never start (`agent::team`).
            let leading: Vec<ConversationId> = state
                .tasks
                .iter()
                .filter_map(|task| task.parent.clone())
                .collect();
            let running = |lane: Lane, state: &State| {
                state
                    .tasks
                    .iter()
                    .filter(|task| task.running && task.lane() == lane)
                    .filter(|task| !leading.contains(&task.status.conversation))
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
                    Lane::Background => {
                        super::capacity::background_room(limit, background, pressure)
                    }
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
            Said::Nobody,
        ))
    }

    pub(crate) fn cancel(self: &Arc<Self>, conversation: &str) {
        // Its helpers stop with it.
        let keys: Vec<u64> = self
            .state()
            .tasks
            .iter()
            .filter(|task| {
                task.status.conversation == conversation
                    || task.parent.as_deref() == Some(conversation)
            })
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
                // Only a schedule the person made authorizes from its words;
                // one an agent made confirms what it would send or delete.
                let said = if schedule.by_person {
                    Said::Request
                } else {
                    Said::Nobody
                };
                self.submit_as(
                    Request {
                        conversation: None,
                        text: schedule.task,
                        origin: Origin::Background,
                        context: Context::default(),
                    },
                    JobKind::Ask,
                    said,
                )
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

#[cfg(test)]
mod tell_job_tests {
    use super::*;

    fn task(id: &str, title: &str) -> TaskStatus {
        TaskStatus {
            conversation: id.into(),
            origin: Origin::Background,
            title: title.into(),
            state: TaskState::Running,
            step: None,
            windows: Vec::new(),
            colour: 0,
        }
    }

    #[test]
    fn a_job_is_named_by_its_short_id_or_title() {
        let running = [
            task("ebcfca18-8bd1", "WhatsApp starten"),
            task("c9c33480-f74f", "Kraftstoffnotdienst recherchieren"),
        ];
        assert_eq!(
            job_named("[c9c33480]", &running).as_deref(),
            Some("c9c33480-f74f")
        );
        assert_eq!(
            job_named("ebcfca18", &running).as_deref(),
            Some("ebcfca18-8bd1")
        );
        assert_eq!(
            job_named("whatsapp", &running).as_deref(),
            Some("ebcfca18-8bd1")
        );
        assert_eq!(job_named("", &running), None);
        assert_eq!(job_named("Tokio", &running), None);
    }
}
