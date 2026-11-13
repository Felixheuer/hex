//! One task from start to end: builds its prompt (the question, its lane's
//! rules, a Here question's window and context, what else is going on, the
//! desktop's guide), gives it its tools (the guard's desktop server, a
//! hidden browser, and Agent-Desk's own: the same toolset for every backend,
//! `toolset`), runs it on the chosen backend (`provider`), reports steps
//! and windows, and saves the conversation as it goes, so a restart
//! finds it `Running` and offers to resume it.
//!
//! A Here or Chat agent that finds the request is a job says so
//! (`markers`), and Agent-Desk continues it as a Background job: a planning turn
//! first (`plan.md` in its own `workspace`), one batched question if it
//! needs anything, then the work with file and shell tools scoped to that
//! folder.
//!
//! OWNER: brain.

mod rules;
pub use rules::*;
mod models;
pub use models::*;
mod prompt;
pub use prompt::*;
pub mod blackboard;
pub mod goal;
pub mod handoff;
pub mod journal;
pub mod long;
pub mod markers;
pub mod merge;
pub mod recovery;
pub mod rollover;
pub mod team;
pub mod toolset;
pub mod watchdog;
pub mod workspace;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crate::app_settings::{AssistantBackend, AssistantSettings};

use super::guard::TaskSpec;
use super::provider::{self, Control, Desktop, Report, Run, limit};
use super::store::{self, Store, Stored};
use super::types::{
    ConversationId, Event, Lane, Message, Origin, Outcome as Ended, Part, Request, Role,
};

/// One task as the runner hands it over.
#[derive(Clone, Debug)]
pub struct Job {
    pub conversation: ConversationId,
    pub request: Request,
    pub settings: AssistantSettings,
    pub kind: JobKind,
    /// Other tasks running and recent conversations, for a new
    /// conversation's first prompt (`service::context`).
    pub digest: Option<String>,
    /// A job's folder: its files, plan, and report. `None` for a quick
    /// question or chat.
    pub workspace: Option<PathBuf>,
    /// What the person authorized for this task (`autonomy`), for its
    /// prompt; the proxies enforce the file.
    pub authorized: crate::assistant::autonomy::Authorized,
    /// The person's memory note, when memory is on.
    pub memory: Option<String>,
    /// What the memory graph knows about the people, companies, and
    /// projects the request's words name (`knowledge::recall`). It goes
    /// with the request's message, never the system prompt, which stays
    /// the same whatever the words ([`system_for`]).
    pub recall: Option<String>,
    /// The job this one helps (`agent::team`): its lead's title.
    pub helps: Option<String>,
    /// The project it runs in (`projects`): its soul's relevant part goes
    /// with the request's message, never the system prompt.
    pub project: Option<crate::assistant::projects::ForTask>,
}

/// Why this run of a conversation happens.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum JobKind {
    /// A new question (or follow-up).
    #[default]
    Ask,
    /// The person spoke into the running task; it was stopped and goes on
    /// with `request.text` as its new instruction.
    Steer {
        /// The step it was on.
        doing: Option<String>,
        /// Its last few steps.
        steps: Vec<String>,
    },
    /// The same question again after a usage limit lifted.
    AfterLimit,
    /// The same question again after Agent-Desk quit or crashed while it ran.
    Resume,
    /// A job's first run: plan, ask what's missing, then do it.
    Job { title: String },
    /// The person answered the job's question (`request.text`).
    Answer { question: String },
    /// The person answered a confirmation; `allowed` when it was a yes.
    Confirm { question: String, allowed: bool },
}

/// How a task ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Answer(String),
    Failed(String),
    Cancelled,
    /// Stopped for a new instruction; a `Steer` run goes on.
    Steered,
    /// The backend hit a usage limit; the conversation is kept running so
    /// an `AfterLimit` run can go on once it lifts.
    Limited {
        reason: String,
        until_ms: Option<u64>,
    },
    /// The job asks the person this and waits for the answer; the
    /// conversation is kept running.
    Question(String),
    /// The request is a job: Agent-Desk continues it in the background. `reply` is
    /// the short line said meanwhile ("On it: …").
    Escalate {
        title: String,
        reply: String,
        /// Further jobs to start beside it: (title, what each does).
        more: Vec<(String, String)>,
        /// What the first job does, when it isn't the whole request (the
        /// request held several jobs, handed on with `hex_start_jobs`).
        first_task: Option<String>,
    },
    /// Agent-Desk refused a consequential action; the person's yes to `question`
    /// allows that kind of action for the task.
    Confirm {
        category: crate::assistant::autonomy::Category,
        question: String,
    },
}

/// What a restarted task is told before anything else.
pub const RESUME_NOTE: &str = "You were interrupted by a restart. Check what was already done (take a fresh look; a job reads its plan.md, journal.md, and report.md first) before continuing where you left off; never repeat a send/submit you may already have done.";

/// What a task is told after waiting out a usage limit.
pub const LIMIT_NOTE: &str = "You were paused by a usage limit and may go on now. Check what was already done (take a fresh look) before continuing; never repeat a send/submit you may already have done.";

/// Runs `job` to its end: saves the question, runs the backend, saves the
/// answer (or the failure), and reports `Thinking`, `Step`, `Window`,
/// `AnswerDelta`, and `ConversationsChanged` through `emit` on the way.
pub fn run(job: &Job, control: &Control, emit: &mut dyn FnMut(Event)) -> Outcome {
    let store = match Store::open() {
        Ok(store) => store,
        Err(error) => {
            return Outcome::Failed(format!("Agent-Desk can't save conversations: {error:#}"));
        }
    };
    run_in(&store, job, control, emit)
}

/// What a task's run needs of the world: its backend, a clock, waiting, and
/// whether the network is there. The long-job simulation replaces it.
pub trait Engine {
    /// Runs `run` on its backend until it answers, fails, or is cancelled.
    fn run(
        &self,
        run: &Run,
        control: &Control,
        report: &mut dyn FnMut(Report),
    ) -> color_eyre::Result<provider::Finished>;

    fn now_ms(&self) -> u64 {
        store::now_ms()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn online(&self) -> bool {
        recovery::online()
    }
}

/// The real backends, clock, and network.
pub struct System;

impl Engine for System {
    fn run(
        &self,
        run: &Run,
        control: &Control,
        report: &mut dyn FnMut(Report),
    ) -> color_eyre::Result<provider::Finished> {
        provider::run(run, control, report)
    }
}

/// `run`, with conversations in `store`.
pub fn run_in(store: &Store, job: &Job, control: &Control, emit: &mut dyn FnMut(Event)) -> Outcome {
    run_with(&System, store, job, control, emit)
}

/// Waits until `engine` is online again, at most `most`; `false` when it
/// wasn't in time or the task was stopped.
fn wait_online(engine: &dyn Engine, control: &Control, most: Duration) -> bool {
    let until = engine.now_ms() + most.as_millis() as u64;
    loop {
        if control.cancelled() {
            return false;
        }
        if engine.online() {
            return true;
        }
        if engine.now_ms() >= until {
            return false;
        }
        engine.sleep(recovery::POLL);
    }
}

/// `run_in` on `engine`. A supervised job (one with a workspace) also
/// keeps its journal, goes on in a fresh session when its session grows
/// too large (`rollover`), and gets past a lost network or an ended backend
/// (`recovery`).
pub fn run_with(
    engine: &dyn Engine,
    store: &Store,
    job: &Job,
    control: &Control,
    emit: &mut dyn FnMut(Event),
) -> Outcome {
    let id = &job.conversation;
    let request = &job.request;
    let mut stored = store
        .load(id)
        .unwrap_or_else(|| Stored::new(id, request.origin, &request.text));
    let asks = match &job.kind {
        JobKind::Ask | JobKind::Answer { .. } | JobKind::Confirm { .. } => true,
        // Written to the conversation when it was said (`service`), so it
        // sits where the person said it.
        JobKind::Steer { .. } => !said_lately(&stored, &request.text, store::now_ms()),
        // A job started as one keeps its question; an escalated one has it,
        // wherever later messages put it (they may come before it starts).
        JobKind::Job { .. } => !said_before(&stored, &request.text),
        JobKind::AfterLimit | JobKind::Resume => false,
    };
    // What came before this run's own message.
    let mut history = stored.conversation.messages.clone();
    let mut image = None;
    if asks {
        let mut parts = vec![Part::Text {
            text: request.text.clone(),
        }];
        if let Some(screenshot) = &request.context.screenshot {
            // The full-resolution capture, so Chats shows it sharp.
            match store.save_attachment(id, screenshot.full.as_deref().unwrap_or(&screenshot.png)) {
                Ok(file) => {
                    image = store.attachment_path(id, &file);
                    parts.push(Part::Screenshot { file });
                }
                Err(error) => tracing::warn!(%error, "couldn't keep the question's screenshot"),
            }
        }
        stored.push(Message {
            role: Role::User,
            parts,
            at_ms: store::now_ms(),
        });
    } else if history.last().is_some_and(|last| last.role == Role::User) {
        history.pop();
    }
    stored.set_running();
    if job.workspace.is_some() {
        stored.workspace = job.workspace.clone();
    }
    if let Some(project) = &job.project {
        stored.project = Some(project.id.clone());
    }
    if let Err(error) = store.save(&stored) {
        return Outcome::Failed(format!("{error:#}"));
    }
    emit(Event::ConversationsChanged);

    let backend = job.settings.backend;
    let key = provider::session_key(backend, job.request.origin.lane());
    let key = key.as_str();
    // A hand-off left by a run that was stopped before it ended isn't this
    // run's to start.
    handoff::clear(&job.conversation);
    let mut resume = stored.sessions.get(key).cloned();
    let mut run = match prepare(job, &history, resume.clone(), image) {
        Ok(run) => run,
        Err(message) => return finish(store, &mut stored, Vec::new(), Err(message), emit),
    };
    // A saved session keeps the rules it started with: when Agent-Desk's rules
    // changed (an update), start a new one that is told the conversation.
    let rules_key = format!("{key}.rules");
    let rules = rules_fingerprint(job, &run);
    if resume.is_some() && stored.sessions.get(&rules_key) != Some(&rules) {
        tracing::info!("the assistant's rules changed; starting a new session");
        resume = None;
        stored.sessions.remove(key);
        run.resume = None;
        run.prompt = message(job, &history, true);
    }
    stored.sessions.insert(rules_key, rules);

    // A job keeps its journal and goes on for hours (`long`): back after a
    // restart or a usage limit, it is told where it was, in a fresh session
    // when its saved one was already too large.
    let mut long = job.workspace.as_deref().map(|workspace| {
        long::Long::open(job, workspace, rollover::Limits::default(), engine.now_ms())
    });
    if let Some(long) = long.as_mut()
        && matches!(job.kind, JobKind::Resume | JobKind::AfterLimit)
    {
        let now = engine.now_ms();
        match long.saved_too_large().filter(|_| run.resume.is_some()) {
            Some(why) => {
                stored.sessions.remove(key);
                resume = None;
                run.resume = None;
                run.prompt = long.roll_over(job, &why, now);
            }
            None => run.prompt = long.go_on(&run.prompt, now),
        }
    }

    // A job works until its goal is met (`goal`): its in-between answers
    // aren't streamed, and an independent check decides when it is done.
    let supervised = job.workspace.is_some();
    let mut progress = goal::Progress::default();
    let mut wrapping_up = false;
    let mut first = true;
    loop {
        let mut seen = Seen {
            quiet: supervised,
            ..Seen::default()
        };
        emit(Event::Thinking {
            conversation: id.clone(),
        });
        let stalls = control.stalls();
        let rotations = control.rotations();
        if let Some(long) = long.as_mut() {
            long.meter.start(engine.now_ms());
        }
        let mut result = run_backend(
            engine,
            &run,
            key,
            control,
            id,
            emit,
            &mut seen,
            store,
            &mut stored,
            long.as_mut(),
        );
        // A saved session the backend no longer has: start a new one that is
        // told the conversation so far.
        if first
            && let Err(error) = &result
            && control.stalls() == stalls
            && control.rotations() == rotations
            && resume.is_some()
            && !seen.streamed
            && seen.actions.is_empty()
            && !control.cancelled()
            && provider::escalated(error).is_none()
            && limit::detect(&format!("{error:#}")).is_none()
        {
            tracing::info!(error = %format!("{error:#}"), "the saved session didn't continue; starting a new one");
            run.resume = None;
            stored.sessions.remove(key);
            run.prompt = message(job, &history, true);
            result = run_backend(
                engine,
                &run,
                key,
                control,
                id,
                emit,
                &mut seen,
                store,
                &mut stored,
                long.as_mut(),
            );
        }
        // Two models: a quick turn the fast model found hard (many tool
        // calls), failed, or got stuck on (`watchdog`) goes on with the
        // strong one, in its session.
        if run.escalate_after.is_some()
            && let Err(error) = &result
            && !control.cancelled()
            && should_escalate(error)
            && let Some(session) = stored.sessions.get(key).cloned()
        {
            tracing::info!(error = %format!("{error:#}"), "the strong model takes the quick turn over");
            let strong = escalated(&run, job, Some(session));
            result = run_backend(
                engine,
                &strong,
                key,
                control,
                id,
                emit,
                &mut seen,
                store,
                &mut stored,
                None,
            );
        }
        // Stuck, and not saved by the strong model: a plain Try again.
        if result.is_err() && control.stalls() > stalls && !control.cancelled() {
            result = Err(color_eyre::eyre::eyre!(watchdog::STALLED));
        }
        first = false;
        let actions = std::mem::take(&mut seen.actions);
        if control.steered() {
            keep_actions(store, &mut stored, actions);
            return Outcome::Steered;
        }
        if let Some(reason) = control.failure() {
            return finish(store, &mut stored, actions, Err(reason), emit);
        }
        if control.cancelled() {
            let _ = finish(store, &mut stored, actions, Err("Stopped.".into()), emit);
            stored.set_outcome(Ended::Cancelled);
            let _ = store.save(&stored);
            return Outcome::Cancelled;
        }
        // The session grew too large and was ended between two steps: the
        // job goes on in a fresh one, seeded with its plan and journal (an
        // answer that came first stands).
        let rotated = control.rotations() > rotations;
        if let Some(long) = long.as_mut()
            && let Some(why) = rotated.then(|| long.due.take()).flatten()
            && result.is_err()
        {
            let why = why.words();
            keep_actions(store, &mut stored, actions);
            stored.sessions.remove(key);
            if let Err(error) = store.save(&stored) {
                tracing::warn!(%error, "couldn't forget the job's old session");
            }
            run.prompt = long.roll_over(job, &why, engine.now_ms());
            run.resume = None;
            run.image = None;
            continue;
        }
        let result = result
            .map(|finished| markers::strip_understood(&finished.text))
            .map_err(|error| format!("{error:#}"));
        if let Err(message) = &result
            && let Some(limit) = limit::detect(message)
        {
            if job.settings.retry_on_limit {
                keep_actions(store, &mut stored, actions);
                if let Some(long) = long.as_mut() {
                    long.journal.note(
                        engine.now_ms(),
                        journal::Kind::Waiting,
                        &format!("usage limit: {}", limit.reason),
                    );
                }
                return Outcome::Limited {
                    reason: limit.reason,
                    until_ms: limit.until_ms,
                };
            }
            let message = format!(
                "{} hit its usage limit: {}",
                provider::name(backend),
                limit.reason
            );
            return finish(store, &mut stored, actions, Err(message), emit);
        }
        // The network went away, or the backend ended without an answer
        // (the computer slept, its screen locked): a job waits for the
        // network and goes on where it was, unless it keeps failing.
        if let Some(long) = long.as_mut()
            && let Err(message) = &result
            && let failure = recovery::classify(message)
            && failure != recovery::Failure::Other
            && long.retries < recovery::MAX_RETRIES
        {
            long.retries += 1;
            keep_actions(store, &mut stored, actions);
            long.journal.note(
                engine.now_ms(),
                journal::Kind::Failed,
                &format!(
                    "the backend's turn ended: {}",
                    crate::assistant::store::shorten(message, 160)
                ),
            );
            if failure == recovery::Failure::Network {
                long.journal
                    .note(engine.now_ms(), journal::Kind::Waiting, "for the network");
                let back = wait_online(engine, control, recovery::MAX_OFFLINE);
                if control.steered() {
                    return Outcome::Steered;
                }
                if let Some(reason) = control.failure() {
                    return finish(store, &mut stored, Vec::new(), Err(reason), emit);
                }
                if control.cancelled() {
                    let _ = finish(store, &mut stored, Vec::new(), Err("Stopped.".into()), emit);
                    stored.set_outcome(Ended::Cancelled);
                    let _ = store.save(&stored);
                    return Outcome::Cancelled;
                }
                if !back {
                    return finish(
                        store,
                        &mut stored,
                        Vec::new(),
                        Err(format!("The network stayed away: {message}")),
                        emit,
                    );
                }
                long.journal
                    .note(engine.now_ms(), journal::Kind::Back, "the network is back");
            }
            let now = engine.now_ms();
            run.resume = stored.sessions.get(key).cloned();
            run.prompt = match &run.resume {
                Some(_) => long.go_on(recovery::note(failure), now),
                None => long.roll_over(job, "the last session ended before it was saved", now),
            };
            run.image = None;
            continue;
        }
        // Jobs handed on with `hex_start_jobs` (the deterministic way), or an
        // answer that claims the background without having started anything
        // (the safety net): either way the jobs start.
        // Instructions for running jobs (`hex_tell_job`) go on now; jobs
        // to start are below.
        let handoff = handoff::take(&job.conversation).and_then(|mut handoff| {
            for (to, text) in std::mem::take(&mut handoff.tells) {
                crate::assistant::service::tell_job(&to, &told_words(job, &text));
            }
            (!handoff.jobs.is_empty()).then_some(handoff)
        });
        if job.workspace.is_none()
            && let Ok(text) = &result
            && let Some(outcome) = handed_on(job, text, handoff)
        {
            let reply = match &outcome {
                Outcome::Escalate { reply, .. } => reply.clone(),
                _ => String::new(),
            };
            keep_reply(store, &mut stored, actions, &reply, emit);
            return outcome;
        }
        if let Ok(text) = &result
            && let Some(marker) = markers::parse(text)
        {
            match marker {
                markers::Marker::Question(question) => {
                    keep_reply(store, &mut stored, actions, &question, emit);
                    return Outcome::Question(question);
                }
                markers::Marker::Confirm { category, question } => {
                    keep_reply(store, &mut stored, actions, &question, emit);
                    return Outcome::Confirm { category, question };
                }
                markers::Marker::Job { title, reply, more } if job.workspace.is_none() => {
                    handoff::clear(&job.conversation);
                    keep_reply(store, &mut stored, actions, &reply, emit);
                    return Outcome::Escalate {
                        title,
                        reply,
                        more,
                        first_task: None,
                    };
                }
                // Already a job: go on as if it had answered.
                markers::Marker::Job { reply, .. } => {
                    return finish(store, &mut stored, actions, Ok(reply), emit);
                }
            }
        }
        let steps = job
            .workspace
            .as_deref()
            .map_or(0, |workspace| workspace::plan(workspace).len());
        if let Some(long) = long.as_mut() {
            long.journal.milestones(engine.now_ms());
        }
        let answer = match &result {
            Ok(answer) if supervised && !wrapping_up && goal::worth_checking(steps) => {
                answer.clone()
            }
            _ => {
                if let Some(long) = long.as_mut() {
                    long.done(
                        engine.now_ms(),
                        if result.is_ok() { "answered" } else { "failed" },
                    );
                }
                return finish(store, &mut stored, actions, result, emit);
            }
        };
        let verdict = check(engine, &run, job, &answer, control, store, &mut stored, key);
        if let Some(long) = long.as_mut() {
            let found = match &verdict {
                goal::Verdict::Verified => "verified".to_owned(),
                goal::Verdict::NotDone(gaps) => format!(
                    "gaps: {}",
                    crate::assistant::store::shorten(&gaps.replace('\n', "; "), 200)
                ),
                goal::Verdict::Unclear => "no verdict".to_owned(),
            };
            long.journal
                .note(engine.now_ms(), journal::Kind::Checked, &found);
        }
        // Stopped, steered, or out of time while it was checked: the job's
        // own answer is what there is.
        if control.steered() {
            keep_actions(store, &mut stored, actions);
            return Outcome::Steered;
        }
        if control.failure().is_some() {
            return finish(store, &mut stored, actions, result, emit);
        }
        if control.cancelled() {
            let _ = finish(store, &mut stored, actions, Err("Stopped.".into()), emit);
            stored.set_outcome(Ended::Cancelled);
            let _ = store.save(&stored);
            return Outcome::Cancelled;
        }
        let next = progress.next(verdict);
        let wraps = matches!(next, goal::Next::WrapUp(_));
        tracing::info!(
            round = progress.rounds,
            stale = progress.stale,
            done = next == goal::Next::Done,
            "job checked"
        );
        match next {
            goal::Next::Done => {
                if let Some(long) = long.as_mut() {
                    long.done(engine.now_ms(), "answered");
                }
                return finish(store, &mut stored, actions, result, emit);
            }
            goal::Next::Continue(mut instruction) | goal::Next::WrapUp(mut instruction) => {
                wrapping_up = wraps;
                keep_actions(store, &mut stored, actions);
                // Its time, and what Agent-Desk found since (`long`).
                if let Some(long) = long.as_mut() {
                    instruction.push_str("\n\n");
                    instruction.push_str(&long.reminder(engine.now_ms()));
                }
                run.prompt = instruction;
                run.resume = stored.sessions.get(key).cloned();
                run.image = None;
            }
        }
    }
}

/// The independent check of a job's answer (`goal`): a fresh session of
/// the same backend with the same tools, told to change nothing, whose
/// steps are neither shown nor kept. Its verdict is recorded in the
/// workspace for the status Agent-Desk tells on request.
#[allow(clippy::too_many_arguments)]
fn check(
    engine: &dyn Engine,
    run: &Run,
    job: &Job,
    answer: &str,
    control: &Control,
    store: &Store,
    stored: &mut Stored,
    key: &str,
) -> goal::Verdict {
    let mut checker = run.clone();
    checker.resume = None;
    checker.image = None;
    checker.keep_warm = None;
    checker.history = Vec::new();
    // The check only looks: its desktop reads but never acts, its browsers
    // never send, post, or pay, its shell is sandboxed without the network
    // even in Yolo, and it has none of Agent-Desk's own tools (no helpers).
    checker.unrestricted = false;
    checker.tools = None;
    if let Some(desktop) = checker.desktop.as_mut() {
        desktop.server = crate::assistant::guard::observe_only(&desktop.server);
    }
    if let Some(browser) = checker.browser.as_mut()
        && browser.args.first().map(String::as_str) == Some("assistant")
        && !browser.args.iter().any(|arg| arg == "--observe")
    {
        browser.args.push("--observe".into());
    }
    checker.prompt = goal::check_prompt(&job.request.text, answer);
    let check_key = format!("{key}.check");
    let mut seen = Seen {
        quiet: true,
        ..Seen::default()
    };
    let id = job.conversation.clone();
    let result = run_backend(
        engine,
        &checker,
        &check_key,
        control,
        &id,
        &mut |_| {},
        &mut seen,
        store,
        stored,
        None,
    );
    stored.sessions.remove(&check_key);
    let verdict = match result {
        Ok(finished) => goal::verdict(&finished.text),
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "a job's check failed; its answer stands");
            goal::Verdict::Unclear
        }
    };
    if let (Some(workspace), Some(record)) = (&job.workspace, goal::check_record(&verdict))
        && let Err(error) = std::fs::write(workspace.join(goal::CHECK_FILE), record)
    {
        tracing::debug!(%error, "couldn't record the job's check");
    }
    verdict
}

/// A short fingerprint of `job`'s rules, to tell when a saved session's
/// rules are out of date: its whole system prompt ([`system_for`], with
/// `run`'s desktop and browser) but the memory note, which grows from
/// request to request without changing the rules. What only the request
/// decides is in its message, not here.
fn rules_fingerprint(job: &Job, run: &Run) -> String {
    use std::hash::{Hash, Hasher};
    let without_note = Job {
        memory: None,
        ..job.clone()
    };
    let rules = system_for(&without_note, run.desktop.as_ref(), run.browser.is_some());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rules.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Whether the person already said `text` in `stored` (a job's request that
/// escalated from a question, or that more was added to while it waited).
fn said_before(stored: &Stored, text: &str) -> bool {
    let text = text.trim();
    stored.conversation.messages.iter().any(|message| {
        message.role == Role::User && {
            let said = message.text();
            let said = said.trim();
            !said.is_empty()
                && (said == text || (said.chars().count() >= 20 && text.starts_with(said)))
        }
    })
}

/// How recent a message must be to count as the one a steer carries.
const STEER_WRITTEN_MS: u64 = 10 * 60 * 1000;

/// Whether `text` is among the person's messages of the last minutes: a
/// steer written to the conversation when it was said.
fn said_lately(stored: &Stored, text: &str, now: u64) -> bool {
    let text = text.trim();
    stored.conversation.messages.iter().rev().any(|message| {
        message.role == Role::User
            && now.saturating_sub(message.at_ms) <= STEER_WRITTEN_MS
            && message.text().trim() == text
    })
}

/// The escalation a Here or Chat run's answer `text` asks for: the jobs it
/// handed on with `hex_start_jobs`, else, when it only *says* the work runs
/// in the background (no tool, no marker), the whole request as one job.
fn handed_on(job: &Job, text: &str, handoff: Option<handoff::Handoff>) -> Option<Outcome> {
    // The line said meanwhile: the tool's `say`, else the answer (without
    // a marker the agent may have added too).
    let spoken = |say: Option<String>| {
        say.filter(|say| !say.trim().is_empty())
            .unwrap_or_else(|| match markers::parse(text) {
                Some(markers::Marker::Job { reply, .. }) => reply,
                _ => text.trim().to_owned(),
            })
    };
    if let Some(handoff) = handoff {
        let mut jobs = handoff.jobs.into_iter();
        let first = jobs.next()?;
        let more: Vec<(String, String)> = jobs.map(|job| (job.title, job.task)).collect();
        // One job is the request itself; several split it.
        let first_task = (!more.is_empty()).then_some(first.task);
        return Some(Outcome::Escalate {
            title: first.title,
            reply: spoken(handoff.say),
            more,
            first_task,
        });
    }
    if markers::parse(text).is_none() && crate::assistant::route::claims_background(text) {
        tracing::info!("an answer claimed the background without a job; starting one");
        return Some(Outcome::Escalate {
            title: crate::assistant::store::shorten(&job.request.text, 60),
            reply: text.trim().to_owned(),
            more: Vec::new(),
            first_task: None,
        });
    }
    None
}

/// What a running job is told when the person's turn passes something on
/// with `hex_tell_job`: the person's own words, which the job's conversation
/// keeps and shows as theirs, never the agent's paraphrase of them ("The
/// person asks: …"). A job telling its helpers passes its own `text`.
fn told_words(job: &Job, text: &str) -> String {
    let said = job.request.text.trim();
    let persons_turn = job.workspace.is_none()
        && job.request.origin != Origin::Background
        && job.kind == JobKind::Ask
        && !said.is_empty();
    if persons_turn {
        said.to_owned()
    } else {
        text.to_owned()
    }
}

/// What a backend run reported that the conversation keeps.
#[derive(Default)]
struct Seen {
    /// Desktop actions done, as the transcript records them.
    actions: Vec<String>,
    streamed: bool,
    /// Keeps markers out of the streamed answer.
    gate: markers::Gate,
    /// Streams nothing: a job's in-between answer, or its check.
    quiet: bool,
}

/// Saves what the assistant said while the conversation goes on (a job's
/// question, or "On it" before a job): its actions and that text.
fn keep_reply(
    store: &Store,
    stored: &mut Stored,
    actions: Vec<String>,
    text: &str,
    emit: &mut dyn FnMut(Event),
) {
    let mut parts: Vec<Part> = actions
        .into_iter()
        .map(|description| Part::Action { description })
        .collect();
    if !text.trim().is_empty() {
        parts.push(Part::Text {
            text: text.trim().to_owned(),
        });
    }
    if !parts.is_empty() {
        stored.push(Message {
            role: Role::Assistant,
            parts,
            at_ms: store::now_ms(),
        });
    }
    if let Err(error) = store.save(stored) {
        tracing::warn!(%error, "couldn't save the assistant's reply");
    }
    emit(Event::ConversationsChanged);
}

/// Runs the backend once, turning its reports into events, and saves its
/// session the moment it has one, so a restart can resume it.
/// A supervised job's step reports also feed its `long` state: the
/// session's size (and a fresh session when it is due), milestones, and
/// its browsers' pages.
#[allow(clippy::too_many_arguments)]
fn run_backend(
    engine: &dyn Engine,
    run: &Run,
    key: &str,
    control: &Control,
    id: &ConversationId,
    emit: &mut dyn FnMut(Event),
    seen: &mut Seen,
    store: &Store,
    stored: &mut Stored,
    mut long: Option<&mut long::Long>,
) -> color_eyre::Result<provider::Finished> {
    let mut report = |report: Report| match report {
        Report::Alive => {}
        Report::Usage(usage) => {
            if let Some(long) = long.as_deref_mut() {
                long.step(usage, engine.now_ms(), control);
            }
        }
        Report::Cost(usd) => {
            if let Some(long) = long.as_deref_mut() {
                long.cost_usd += usd;
            }
        }
        Report::Thinking => emit(Event::Thinking {
            conversation: id.clone(),
        }),
        Report::Step { doing, done } => {
            if !done.is_empty() {
                seen.actions.push(done);
            }
            emit(Event::Step {
                conversation: id.clone(),
                description: doing,
            });
        }
        Report::Window(window) => emit(Event::Window {
            conversation: id.clone(),
            window,
        }),
        Report::Delta(text) => {
            seen.streamed = true;
            let shown = seen.gate.pass(&text);
            // What it understood, before anything it does (`markers`).
            if let Some(text) = seen.gate.take_understood() {
                emit(Event::Understood {
                    conversation: id.clone(),
                    text,
                });
            }
            if let Some(text) = shown
                && !seen.quiet
            {
                emit(Event::AnswerDelta {
                    conversation: id.clone(),
                    text,
                });
            }
        }
        Report::Tool { name, input } => {
            if let Some(acts) = crate::assistant::guard::screen_work(&name) {
                emit(Event::Acting {
                    conversation: id.clone(),
                    looking: !acts,
                });
            }
            // A script reaching an app macOS hasn't asked about: the step
            // says the prompt is coming, so the person allows it.
            if let Some(notice) = crate::desktop::automation::first_use_notice(&input) {
                emit(Event::Step {
                    conversation: id.clone(),
                    description: notice,
                });
            }
            if let Some(long) = long.as_deref_mut() {
                long.tool(&name, &input);
            }
            crate::assistant::audit::record(
                id,
                &crate::assistant::audit::Entry {
                    at_ms: crate::assistant::store::now_ms(),
                    tool: name,
                    input,
                    check: seen.quiet,
                },
            )
        }
        Report::Session(session) => {
            if stored.sessions.get(key) != Some(&session) {
                stored.sessions.insert(key.to_owned(), session);
                if let Err(error) = store.save(stored) {
                    tracing::warn!(%error, "couldn't keep the backend's session");
                }
            }
        }
    };
    // A quick turn that reports nothing for too long is ended (`watchdog`).
    watchdog::watch(control, watchdog::limit(run), |touch| {
        engine.run(run, control, &mut |sign: Report| {
            touch();
            report(sign);
        })
    })
}

/// Keeps what a run did before it stopped to go on later (steered, or
/// waiting out a limit): its actions, as an assistant message.
fn keep_actions(store: &Store, stored: &mut Stored, actions: Vec<String>) {
    if !actions.is_empty() {
        stored.push(Message {
            role: Role::Assistant,
            parts: actions
                .into_iter()
                .map(|description| Part::Action { description })
                .collect(),
            at_ms: store::now_ms(),
        });
    }
    if let Err(error) = store.save(stored) {
        tracing::warn!(%error, "couldn't save the task's progress");
    }
}

/// Saves the assistant's message and says how the task ended.
fn finish(
    store: &Store,
    stored: &mut Stored,
    actions: Vec<String>,
    result: Result<String, String>,
    emit: &mut dyn FnMut(Event),
) -> Outcome {
    let mut parts: Vec<Part> = actions
        .into_iter()
        .map(|description| Part::Action { description })
        .collect();
    let outcome = match result {
        Ok(text) if !text.trim().is_empty() => {
            parts.push(Part::Text { text: text.clone() });
            Outcome::Answer(text)
        }
        Ok(_) => {
            let message = "The agent finished without an answer.".to_owned();
            parts.push(Part::Error {
                message: message.clone(),
            });
            Outcome::Failed(message)
        }
        Err(message) => {
            parts.push(Part::Error {
                message: message.clone(),
            });
            Outcome::Failed(message)
        }
    };
    stored.push(Message {
        role: Role::Assistant,
        parts,
        at_ms: store::now_ms(),
    });
    stored.set_outcome(match outcome {
        Outcome::Answer(_) => Ended::Answered,
        _ => Ended::Failed,
    });
    if let Err(error) = store.save(stored) {
        tracing::warn!(%error, "couldn't save the assistant's answer");
    }
    emit(Event::ConversationsChanged);
    outcome
}

/// What the guard needs to know about `job`.
pub fn task_spec(job: &Job) -> TaskSpec {
    let lane = job.request.origin.lane();
    TaskSpec {
        task: job.conversation.clone(),
        lane,
        here_window: match lane {
            Lane::Here => job.request.context.window.clone(),
            Lane::Background => None,
        },
        physical_input: match lane {
            Lane::Here => job.settings.here_physical_input,
            Lane::Background => job.settings.background_physical_input,
        },
        blocked_apps: job.settings.blocked_apps.clone(),
        full_control: crate::assistant::full_control(job.settings.full_control, &job.request.text),
        // "Only in the background" keeps the person's windows theirs, not
        // the apps out of reach: they open in windows of the task's own.
        open_apps: job.settings.full_control,
    }
}

/// The backend run for `job`, or why it can't run.
pub fn prepare(
    job: &Job,
    history: &[Message],
    resume: Option<String>,
    image: Option<PathBuf>,
) -> Result<Run, String> {
    let settings = &job.settings;
    let spec = task_spec(job);
    // The desktop only ever through the guard: it is what decides which
    // windows and tools the task has (`guard::proxy::tools`).
    let desktop = if settings.control_desktop {
        let mut desktop = super::guard::mcp_server(&spec)
            .map(Desktop::new)
            .ok_or(NO_DRIVER)?;
        desktop.headed = spec.physical_input;
        desktop.front_window = spec.full_control;
        desktop.open_apps = spec.open_apps;
        Some(desktop)
    } else {
        None
    };
    let lane = spec.lane;
    // Both lanes: a quick "open a browser and go to …" runs in Agent-Desk's own
    // browser (a live picture on the overlay), never in the person's Chrome.
    let browser = (settings.background_browser && (lane == Lane::Background || spec.full_control))
        .then(|| super::browser::mcp_server(&job.conversation))
        .flatten();
    let workdir = match &job.workspace {
        Some(workspace) => workspace.clone(),
        None => workdir().map_err(|error| {
            format!("Agent-Desk can't prepare the assistant's folder: {error:#}")
        })?,
    };
    let continued = resume.is_none() && settings.backend != AssistantBackend::ClaudeApi;
    let keep_warm = (lane == Lane::Here && settings.backend == AssistantBackend::ClaudeCode)
        .then(|| keep_warm_for(settings))
        .flatten();
    let system = system_for(job, desktop.as_ref(), browser.is_some());
    let tools = tools_server(&job.conversation, tool_role(job), job.workspace.as_deref());
    Ok(Run {
        backend: settings.backend,
        model: model(job),
        effort: effort(job),
        escalate_after: escalate_after(job),
        system,
        prompt: message(job, history, continued),
        image,
        resume,
        history: history.to_vec(),
        desktop,
        browser,
        workdir,
        quick: lane == Lane::Here,
        keep_warm,
        workspace: job.workspace.clone(),
        tools,
        unrestricted: job.workspace.is_some()
            && settings.autonomy == crate::assistant::autonomy::Autonomy::Yolo,
        backend_env: (settings.backend == AssistantBackend::OpenCode).then(|| {
            crate::assistant::backends::agent_env_for(AssistantBackend::OpenCode, settings)
        }),
        provider_config: (settings.backend == AssistantBackend::OpenCode)
            .then(|| crate::assistant::backends::opencode_provider_config(settings))
            .flatten(),
    })
}

/// Why a task that may use apps can't.
const NO_DRIVER: &str = "Cua Driver, the desktop driver, isn't installed, so the assistant can't use apps. Install it in Assistant settings, or turn off “Use apps”.";

/// Whom Agent-Desk's tools serve for `job` (`tools::Role`): a background job
/// reads pages and files the person never saw, with or without a workspace
/// (one couldn't be made), so it never gets the person's own turn's tools;
/// a helper is a job without a team of its own.
pub(crate) fn tool_role(job: &Job) -> crate::assistant::tools::Role {
    use crate::assistant::tools::Role;
    if job.helps.is_some() {
        Role::Helper
    } else if job.workspace.is_some() || job.request.origin == Origin::Background {
        Role::Job
    } else {
        Role::Turn
    }
}

/// Agent-Desk's own tools for a task (`tools`): memory and schedules for the
/// person's own turn, a team for a job.
fn tools_server(
    task: &str,
    role: crate::assistant::tools::Role,
    workspace: Option<&std::path::Path>,
) -> Option<crate::assistant::backends::McpServer> {
    let mut args: Vec<String> = [
        "assistant",
        "tools-mcp",
        "--task",
        task,
        "--role",
        role.arg(),
    ]
    .map(str::to_owned)
    .to_vec();
    // A job and its helpers share the lead's board (`blackboard`).
    if role.is_job()
        && let Some(workspace) = workspace
    {
        let helper = role == crate::assistant::tools::Role::Helper;
        args.push("--board".into());
        args.push(
            blackboard::path_for(workspace, helper)
                .display()
                .to_string(),
        );
    }
    Some(crate::assistant::backends::McpServer {
        command: std::env::current_exe().ok()?,
        args,
        path_env: std::env::var("PATH").unwrap_or_default(),
    })
}

/// Starts `job`'s backend ahead of its question (`service::prewarm`).
pub fn prewarm(job: &Job) {
    let resume = Store::open()
        .ok()
        .and_then(|store| store.load(&job.conversation))
        .and_then(|stored| {
            stored
                .sessions
                .get(&provider::session_key(
                    job.settings.backend,
                    job.request.origin.lane(),
                ))
                .cloned()
        });
    let ttl = ahead_for(
        &job.settings,
        job.request.origin.lane(),
        job.request.conversation.is_none(),
    );
    match prepare(job, &[], resume, None) {
        Ok(run) => provider::prewarm(&run, ttl),
        Err(why) => tracing::debug!(why, "nothing to start ahead"),
    }
}

/// The folder every backend runs in: private, outside any project, and the
/// same each time so a backend finds its saved sessions.
fn workdir() -> color_eyre::Result<PathBuf> {
    let dir = store::assistant_dir()?.join("workspace");
    crate::app_paths::create_private_dir(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod long_tests;
#[cfg(test)]
mod tests;
