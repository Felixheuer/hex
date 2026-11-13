//! One task from start to end: builds its prompt (the question, its lane's
//! rules, a Here question's window and context, what else is going on, the
//! desktop's guide), gives it its tools (the guard's desktop server, or
//! agent-desktop directly when there is no guard, and a hidden browser for
//! Background work), runs it on the chosen backend (`provider`), reports
//! steps and windows, and saves the conversation as it goes, so a restart
//! finds it `Running` and offers to resume it.
//!
//! A Here or Chat agent that finds the request is a job says so
//! (`markers`), and Hex continues it as a Background job: a planning turn
//! first (`plan.md` in its own `workspace`), one batched question if it
//! needs anything, then the work with file and shell tools scoped to that
//! folder.
//!
//! OWNER: brain.

mod rules;
pub use rules::*;
pub mod goal;
pub mod handoff;
pub mod markers;
pub mod team;
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
    /// This task's agent id in the desktop session: its own cursor.
    pub agent_id: String,
    /// The multi-agent desktop session, when one is running.
    pub desktop_session: Option<String>,
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
    /// The job this one helps (`agent::team`): its lead's title.
    pub helps: Option<String>,
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
    /// The same question again after Hex quit or crashed while it ran.
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
    /// The request is a job: Hex continues it in the background. `reply` is
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
    /// Hex refused a consequential action; the person's yes to `question`
    /// allows that kind of action for the task.
    Confirm {
        category: crate::assistant::autonomy::Category,
        question: String,
    },
}

/// What a restarted task is told before anything else.
pub const RESUME_NOTE: &str = "You were interrupted by a restart. Check what was already done (take a fresh look; a job reads its plan.md and report.md first) before continuing where you left off; never repeat a send/submit you may already have done.";

/// What a task is told after waiting out a usage limit.
pub const LIMIT_NOTE: &str = "You were paused by a usage limit and may go on now. Check what was already done (take a fresh look) before continuing; never repeat a send/submit you may already have done.";

/// Runs `job` to its end: saves the question, runs the backend, saves the
/// answer (or the failure), and reports `Thinking`, `Step`, `Window`,
/// `AnswerDelta`, and `ConversationsChanged` through `emit` on the way.
pub fn run(job: &Job, control: &Control, emit: &mut dyn FnMut(Event)) -> Outcome {
    let store = match Store::open() {
        Ok(store) => store,
        Err(error) => return Outcome::Failed(format!("Hex can't save conversations: {error:#}")),
    };
    run_in(&store, job, control, emit)
}

/// `run`, with conversations in `store`.
pub fn run_in(store: &Store, job: &Job, control: &Control, emit: &mut dyn FnMut(Event)) -> Outcome {
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
    // A saved session keeps the rules it started with: when Hex's rules
    // changed (an update), start a new one that is told the conversation.
    let rules_key = format!("{key}.rules");
    let rules = rules_fingerprint(&run.system);
    if resume.is_some() && stored.sessions.get(&rules_key) != Some(&rules) {
        tracing::info!("the assistant's rules changed; starting a new session");
        resume = None;
        stored.sessions.remove(key);
        run.resume = None;
        run.prompt = message(job, &history, true);
    }
    stored.sessions.insert(rules_key, rules);

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
        let mut result = run_backend(&run, key, control, id, emit, &mut seen, store, &mut stored);
        // A saved session the backend no longer has: start a new one that is
        // told the conversation so far.
        if first
            && let Err(error) = &result
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
            result = run_backend(&run, key, control, id, emit, &mut seen, store, &mut stored);
        }
        // Two models: a quick turn the fast model found hard (many tool
        // calls) or failed goes on with the strong one, in its session.
        if run.escalate_after.is_some()
            && let Err(error) = &result
            && !control.cancelled()
            && should_escalate(error)
            && let Some(session) = stored.sessions.get(key).cloned()
        {
            tracing::info!(error = %format!("{error:#}"), "the strong model takes the quick turn over");
            let strong = escalated(&run, job, Some(session));
            result = run_backend(
                &strong,
                key,
                control,
                id,
                emit,
                &mut seen,
                store,
                &mut stored,
            );
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
        let result = result
            .map(|finished| finished.text)
            .map_err(|error| format!("{error:#}"));
        if let Err(message) = &result
            && let Some(limit) = limit::detect(message)
        {
            if job.settings.retry_on_limit {
                keep_actions(store, &mut stored, actions);
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
        let answer = match &result {
            Ok(answer) if supervised && !wrapping_up && goal::worth_checking(steps) => {
                answer.clone()
            }
            _ => return finish(store, &mut stored, actions, result, emit),
        };
        let verdict = check(&run, job, &answer, control, store, &mut stored, key);
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
            goal::Next::Done => return finish(store, &mut stored, actions, result, emit),
            goal::Next::Continue(instruction) | goal::Next::WrapUp(instruction) => {
                wrapping_up = wraps;
                keep_actions(store, &mut stored, actions);
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
/// workspace for the status Hex tells on request.
fn check(
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
    // even in Yolo, and it has none of Hex's own tools (no helpers).
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
        &checker,
        &check_key,
        control,
        &id,
        &mut |_| {},
        &mut seen,
        store,
        stored,
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

/// A short fingerprint of a system prompt, to tell when a saved session's
/// rules are out of date. What follows the autonomy section (this request's
/// permissions, the memory note) changes from request to request without
/// changing the rules, so only the text before it counts.
fn rules_fingerprint(system: &str) -> String {
    use std::hash::{Hash, Hasher};
    let rules = system.split("\nAutonomy (").next().unwrap_or(system);
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
    if markers::parse(text).is_none() && handoff::claims_background(text) {
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
#[allow(clippy::too_many_arguments)]
fn run_backend(
    run: &Run,
    key: &str,
    control: &Control,
    id: &ConversationId,
    emit: &mut dyn FnMut(Event),
    seen: &mut Seen,
    store: &Store,
    stored: &mut Stored,
) -> color_eyre::Result<provider::Finished> {
    let mut report = |report: Report| match report {
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
            if let Some(text) = seen.gate.pass(&text)
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
    provider::run(run, control, &mut report)
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
        agent_id: job.agent_id.clone(),
        session: job.desktop_session.clone(),
        here_window: match lane {
            Lane::Here => job.request.context.window.clone(),
            Lane::Background => None,
        },
        physical_input: match lane {
            Lane::Here => job.settings.here_physical_input,
            Lane::Background => job.settings.physical_input,
        },
        blocked_apps: job.settings.blocked_apps.clone(),
        full_control: crate::assistant::full_control(
            job.settings.background_front_window,
            &job.request.text,
        ),
        // "Only in the background" keeps the person's windows theirs, not
        // the apps out of reach: they open in windows of the task's own.
        open_apps: job.settings.background_front_window,
    }
}

/// The model `job` runs on: the Here lane's fast one where the backend
/// takes it, else the backend's own.
pub fn model(job: &Job) -> Option<String> {
    let settings = &job.settings;
    let here = job.request.origin.lane() == Lane::Here;
    let fast = settings
        .here_model
        .as_deref()
        .map(str::trim)
        .filter(|model| here && !model.is_empty())
        .map(str::to_owned);
    match settings.backend {
        AssistantBackend::ClaudeCode => fast.or_else(|| settings.claude_code_model.clone()),
        AssistantBackend::Codex => settings.codex_model.clone(),
        AssistantBackend::OpenCode => crate::assistant::backends::opencode_model(settings),
        AssistantBackend::ClaudeApi => fast.or_else(|| Some(settings.claude_api_model.clone())),
    }
}

/// Whether a fast model's failed turn goes on with the strong one: it
/// handed over ([`provider::Escalate`]) or failed, but not on a usage
/// limit (the strong model shares it) nor when it was stopped.
fn should_escalate(error: &color_eyre::Report) -> bool {
    provider::escalated(error).is_some()
        || (limit::detect(&format!("{error:#}")).is_none() && format!("{error:#}") != "cancelled")
}

/// The model a Here turn on the fast one goes on with when it hands over
/// ([`provider::Escalate`]): Claude Code's own (`None`: its default).
pub fn strong_model(settings: &AssistantSettings) -> Option<String> {
    settings
        .claude_code_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

/// Whether `job` runs on the Here lane's fast model on Claude Code, where
/// a hard turn can hand over to the strong one.
fn on_fast_model(job: &Job) -> bool {
    job.settings.backend == AssistantBackend::ClaudeCode
        && job.request.origin.lane() == Lane::Here
        && model(job).is_some_and(|fast| Some(&fast) != strong_model(&job.settings).as_ref())
}

/// After how many tool calls a Here turn on the fast model hands over to
/// the strong one (`assistant.escalate_after_tools`); `None` never.
pub fn escalate_after(job: &Job) -> Option<usize> {
    let after = job.settings.escalate_after_tools as usize;
    (after > 0 && on_fast_model(job)).then_some(after)
}

/// How hard a run of `job` thinks: the Here lane's effort on Claude Code
/// (`assistant.here_effort`), else the model's own.
pub fn effort(job: &Job) -> Option<String> {
    (job.settings.backend == AssistantBackend::ClaudeCode
        && job.request.origin.lane() == Lane::Here)
        .then(|| job.settings.here_effort.clone())
        .flatten()
        .filter(|effort| !effort.trim().is_empty())
}

/// What the strong model is told when it takes a turn over from the fast
/// one, in the same session.
pub const ESCALATED_NOTE: &str = "You are taking this turn over from a faster model, in the same session: everything above happened. Check where it stands (take a fresh look where it matters), then finish the person's request; never repeat a send or submit that may already have happened. Your answer is read aloud: keep it short and speakable.";

/// The run that hands `run` (a fast model's turn that stopped or failed)
/// to the strong model: the same session, a note to go on.
fn escalated(run: &Run, job: &Job, session: Option<String>) -> Run {
    let mut strong = run.clone();
    strong.model = strong_model(&job.settings);
    strong.effort = None;
    strong.escalate_after = None;
    // Its process has other arguments than the next fast turn's: keeping it
    // would only wait to expire.
    strong.keep_warm = None;
    strong.image = None;
    strong.resume = session;
    strong.prompt = ESCALATED_NOTE.to_owned();
    strong
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
    let desktop = if settings.control_desktop {
        let guard = super::guard::mcp_server(&spec);
        if guard.is_none() {
            log_once("the guard has no MCP server yet; tasks use agent-desktop directly");
        }
        Some(
            Desktop::new(
                guard,
                job.desktop_session.clone(),
                job.agent_id.clone(),
                spec.physical_input,
            )
            .ok_or(
                "agent-desktop isn't installed, so the assistant can't use apps. Install it in Chats › Settings, or turn off “Use apps”.",
            )
            .map(|mut desktop| {
                desktop.front_window = spec.full_control;
                desktop.open_apps = spec.open_apps;
                desktop
            })?,
        )
    } else {
        None
    };
    let lane = spec.lane;
    // Both lanes: a quick "open a browser and go to …" runs in Hex's own
    // browser (a live picture on the overlay), never in the person's Chrome.
    let browser = (settings.background_browser && (lane == Lane::Background || spec.full_control))
        .then(|| super::browser::mcp_server(&job.conversation))
        .flatten();
    let workdir = match &job.workspace {
        Some(workspace) => workspace.clone(),
        None => workdir()
            .map_err(|error| format!("Hex can't prepare the assistant's folder: {error:#}"))?,
    };
    let continued = resume.is_none() && settings.backend != AssistantBackend::ClaudeApi;
    let keep_warm = (lane == Lane::Here && settings.backend == AssistantBackend::ClaudeCode)
        .then(|| keep_warm_for(settings))
        .flatten();
    let system = system_for(job, desktop.as_ref(), browser.is_some());
    let job_role = job_role(job);
    let tools = tools_server(
        &job.conversation,
        if job_role {
            crate::assistant::tools::Role::Job
        } else {
            crate::assistant::tools::Role::Turn
        },
    );
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

/// Whether `job` works as a job: a background job reads pages and files
/// the person never saw, with or without a workspace (one couldn't be
/// made), so it never gets the person's own turn's tools (`tools::Role`).
fn job_role(job: &Job) -> bool {
    job.workspace.is_some() || job.request.origin == Origin::Background
}

/// How long a Here process waits after its answer: for the follow-up
/// window (the next new conversation's is started by `service::stand_by`).
/// `None`: it ends.
fn keep_warm_for(settings: &AssistantSettings) -> Option<Duration> {
    let seconds = u64::from(settings.follow_up_seconds);
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

/// The system prompt of `job`'s run: who the assistant is, its lane's and
/// tools' rules, the routing or job rules, the autonomy mode, and the
/// memory note. Only the settings and the task decide it, never the words
/// of the request, so a backend started ahead of the question (`prewarm`)
/// or kept for the follow-up has exactly it, and the model's prompt cache
/// keeps it from one request to the next. What only the request decides
/// goes with its message ([`request_rules`]).
pub fn system_for(job: &Job, desktop: Option<&Desktop>, browser: bool) -> String {
    let settings = &job.settings;
    let origin = job.request.origin;
    let mut system = system_prompt(origin, desktop, browser);
    match &job.workspace {
        Some(workspace) => {
            system.push_str(&job_rules(
                workspace,
                browser,
                settings.autonomy == crate::assistant::autonomy::Autonomy::Yolo,
            ));
            system.push_str(&project_rules(&settings.projects_folder));
        }
        None if origin != Origin::Background => system.push_str(ROUTING),
        None => {}
    }
    if let Some(lead) = &job.helps {
        system.push_str(&team::helper_rules(lead));
    }
    // The mode's baseline; what the person's words allowed on top of it is
    // the request's (`request_rules`).
    system.push_str(&autonomy_rules(
        settings.autonomy,
        &crate::assistant::autonomy::Authorized::default(),
    ));
    let job_role = job_role(job);
    // A job can't make schedules or remember (`tools::Role::Job`); it still
    // reads the memory note.
    if !job_role {
        system.push_str(TOOLS_RULES);
    }
    system.push_str(crate::assistant::knowledge::RULES);
    system.push_str(rules::BACKGROUND_INPUT_RULES);
    if settings.memory {
        system.push_str(&memory_rules(job.memory.as_deref(), !job_role));
    }
    system
}

/// What only this request decides, sent at the top of its message: the
/// person's language (pages, tools, and Hex's notes are mostly English, so
/// it is named) and, when their words or a yes allowed more than the
/// mode's baseline in the system prompt, what they allowed.
pub fn request_rules(job: &Job) -> String {
    let mode = job.settings.autonomy;
    let mut rules = language_rule(&job.request.text).trim().to_owned();
    let allowed = autonomy_rules(mode, &job.authorized);
    if allowed != autonomy_rules(mode, &crate::assistant::autonomy::Authorized::default()) {
        rules.push_str("\nFor this request: ");
        rules.push_str(allowed.trim());
    }
    rules
}

/// The message a run sends: [`request_rules`], then [`prompt`] with the
/// desktop as it is now.
fn message(job: &Job, history: &[Message], with_history: bool) -> String {
    format!(
        "{}\n\n{}",
        request_rules(job),
        prompt_now(job, history, with_history)
    )
}

/// Hex's own tools for a task (`tools`): memory and schedules for the
/// person's own turn, a team for a job.
fn tools_server(
    task: &str,
    role: crate::assistant::tools::Role,
) -> Option<crate::assistant::backends::McpServer> {
    Some(crate::assistant::backends::McpServer {
        command: std::env::current_exe().ok()?,
        args: [
            "assistant",
            "tools-mcp",
            "--task",
            task,
            "--role",
            role.arg(),
        ]
        .map(str::to_owned)
        .to_vec(),
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

/// How long a backend started ahead waits for its request.
const AHEAD: Duration = Duration::from_secs(60);

/// How long a backend started ahead for the next `lane` request (for a
/// `new` conversation or not) waits for it: a minute, and a new Here
/// conversation's `assistant.warm_minutes` (the always-warm cap).
pub fn ahead_for(settings: &AssistantSettings, lane: Lane, new: bool) -> Duration {
    let warm = Duration::from_secs(u64::from(settings.warm_minutes) * 60);
    if lane == Lane::Here && new {
        warm.max(AHEAD)
    } else {
        AHEAD
    }
}

fn log_once(message: &'static str) {
    static LOGGED: OnceLock<()> = OnceLock::new();
    if LOGGED.set(()).is_ok() {
        tracing::info!("{message}");
    }
}

/// The folder every backend runs in: private, outside any project, and the
/// same each time so a backend finds its saved sessions.
fn workdir() -> color_eyre::Result<PathBuf> {
    let dir = store::assistant_dir()?.join("workspace");
    crate::app_paths::create_private_dir(&dir)?;
    Ok(dir)
}

/// The message a run sends: the conversation so far when the backend has no
/// session for it (`with_history`), what else is going on for a new
/// conversation, the Here lane's window and what was on screen, then the
/// question, the new instruction (steering), or the note to go on.
pub fn prompt(job: &Job, history: &[Message], with_history: bool) -> String {
    prompt_with(job, history, with_history, None)
}

/// [`prompt`] with the desktop as it is now (`computer::now`), read fresh
/// for each run of a task that may use the desktop.
fn prompt_now(job: &Job, history: &[Message], with_history: bool) -> String {
    let now = job
        .settings
        .control_desktop
        .then(|| super::computer::now::for_prompt(&job.settings.blocked_apps))
        .flatten();
    prompt_with(job, history, with_history, now.as_deref())
}

/// [`prompt`], with `desktop_now` (the running apps and their windows)
/// before what was on screen and the question.
pub fn prompt_with(
    job: &Job,
    history: &[Message],
    with_history: bool,
    desktop_now: Option<&str>,
) -> String {
    let request = &job.request;
    let mut prompt = String::new();
    if with_history && !history.is_empty() {
        prompt.push_str("The conversation so far:\n");
        for message in history {
            let text = message.text();
            if text.trim().is_empty() {
                continue;
            }
            let who = if message.role == Role::User {
                "Person"
            } else {
                "You"
            };
            prompt.push_str(&format!("{who}: {}\n", text.trim()));
        }
        prompt.push('\n');
    }
    if history.is_empty()
        && let Some(digest) = job.digest.as_deref().filter(|digest| !digest.is_empty())
    {
        prompt.push_str(digest.trim_end());
        prompt.push_str("\n\n");
    }
    if let Some(now) = desktop_now.filter(|now| !now.trim().is_empty()) {
        prompt.push_str(now.trim_end());
        prompt.push_str("\n\n");
    }
    let context = &request.context;
    let mut seen = Vec::new();
    if let Some(app) = context
        .frontmost_app
        .as_deref()
        .filter(|app| !app.is_empty())
    {
        seen.push(format!("Frontmost app: {app}"));
    }
    if let Some(title) = context
        .window_title
        .as_deref()
        .filter(|title| !title.is_empty())
    {
        seen.push(format!("Its window: {title}"));
    }
    if request.origin.lane() == Lane::Here
        && let Some(window) = &context.window
    {
        let id = window
            .window_id
            .as_deref()
            .map(|id| format!(" (window id {id})"))
            .unwrap_or_default();
        // With full control every window is in reach; without it, only
        // the one in front.
        let full =
            crate::assistant::full_control(job.settings.background_front_window, &request.text);
        seen.push(if full {
            format!("The window in front: {}{id}", window.app)
        } else {
            format!("Your only window, the one in front: {}{id}", window.app)
        });
    }
    if let Some(display) = context
        .display
        .as_deref()
        .filter(|display| !display.is_empty())
    {
        seen.push(format!("Displays: {display}"));
    }
    if let Some(selected) = context
        .selected_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        seen.push(format!("Selected text:\n<<<\n{}\n>>>", selected.trim()));
    }
    if context.screenshot.is_some() {
        seen.push("A screenshot of the screen is attached.".into());
    }
    if !seen.is_empty() {
        prompt.push_str("What the person has on screen as they ask:\n");
        prompt.push_str(&seen.join("\n"));
        prompt.push_str("\n\n");
    }
    let text = request.text.trim();
    match &job.kind {
        JobKind::Ask => {
            if !prompt.is_empty() {
                prompt.push_str("The person asks: ");
            }
            prompt.push_str(text);
        }
        JobKind::Steer { doing, steps } => {
            let doing = doing.as_deref().unwrap_or("the task");
            prompt.push_str(&format!(
                "The person interrupted you while you were doing this: {doing}"
            ));
            if !steps.is_empty() {
                prompt.push_str(&format!(" (last steps: {})", steps.join("; ")));
            }
            prompt.push_str(&format!(
                ". Their new instruction: “{text}”. Adjust what you are doing; don't redo finished steps."
            ));
        }
        JobKind::AfterLimit => {
            prompt.push_str(&format!("{LIMIT_NOTE}\nThe task was: “{text}”"));
        }
        JobKind::Resume => {
            prompt.push_str(&format!("{RESUME_NOTE}\nThe task was: “{text}”"));
        }
        JobKind::Job { title } => {
            prompt.push_str(&format!(
                "Your job (“{title}”), as the person asked for it: {text}\nStart with the plan."
            ));
        }
        JobKind::Answer { question } => {
            prompt.push_str(&format!(
                "You asked: “{question}”\nThe person answered: “{text}”\nGo on with the job."
            ));
        }
        JobKind::Confirm { question, allowed } => {
            prompt.push_str(&format!(
                "You asked: “{question}”\nThe person answered: “{text}”\n{}",
                if *allowed {
                    "That's a yes: Hex now allows it for this task. Do it, then go on."
                } else {
                    "That's not a yes: skip that action, go on with the rest, and say at the end what you skipped."
                }
            ));
        }
    }
    prompt
}

/// What the person's computer is called in a prompt: "Mac", "Windows PC",
/// or "Linux computer".
pub fn computer_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mac"
    } else if cfg!(windows) {
        "Windows PC"
    } else {
        "Linux computer"
    }
}

/// The language every answer, question, and report is written in: the
/// request's, else this Mac's.
fn language_rule(request: &str) -> String {
    let code = crate::assistant::speech::language_of(request)
        .map(str::to_owned)
        .unwrap_or_else(crate::assistant::speech::system_language);
    let name = match code.as_str() {
        "de" => "German",
        "en" => "English",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "nl" => "Dutch",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "uk" => "Ukrainian",
        "ru" => "Russian",
        "ja" => "Japanese",
        "zh" => "Chinese",
        _ => {
            return "\nWrite every answer, question, and report in the person's language.\n".into();
        }
    };
    format!(
        "\nThe person speaks {name}: write every answer, question, and report to them in {name}, even when pages, tools, and Hex's notes are in English (messages to others in the language the request says).\n"
    )
}

/// The browser rules of a quick (Here) turn.
pub const HERE_BROWSER: &str = "You have your own hidden browser (the browser tools) for a quick look on the web or a page the person asks to open; prefer it over their browser windows. Omit `session`; it starts with the person's accounts when there are any, and `signedIn: true` opens real Chrome for heavy web apps or logins the light one can't handle. Never solve or get around a captcha or a check that you are human: ask the person with a reply starting `HEX_QUESTION:` to solve it in the live picture of that browser on your card.\n";

/// Who the assistant is, how it answers, and its lane's rules for the
/// desktop and the browser.
pub fn system_prompt(origin: Origin, desktop: Option<&Desktop>, browser: bool) -> String {
    let mut prompt = format!(
        "You are Hex's assistant on the person's {}. Hex is a voice app: the person speaks a request and you answer or do it.\n\
         Answer in the language the person used.\n",
        computer_name()
    );
    prompt.push_str(match origin {
        Origin::ActionShortcut => {
            "This is a quick spoken request, often about the window in front of the person: do it or answer it right away. Your answer is shown in a small bubble and read aloud, so keep it short (one to three sentences), plain, and speakable: no markdown, lists, tables, code blocks, links, or emoji. The question was transcribed from speech, so read past small transcription errors.\n"
        }
        Origin::Background => {
            "This is a background task, running beside the person and other tasks while they keep working. Do it, then end with one or two plain sentences saying what you did; no markdown.\n"
        }
        Origin::Chat => {
            "This is a typed chat; it may run beside other tasks. Markdown is fine; be as brief as the question allows.\n"
        }
    });
    // A quick turn's browser is for a quick look or a page the person asks
    // to open (research is a job's, `ROUTING`): the short form.
    if browser && origin.lane() == Lane::Here {
        prompt.push_str(HERE_BROWSER);
    } else if browser {
        prompt.push_str(&format!(
            "You have your own hidden browsers (the browser tools): prefer them for anything on the web (looking things up, web apps, forms), over the person's browser windows. Every browser tool takes `session` (omit it, or \"main\", for the main browser; any other short name opens another, at most {} on this computer). For research, or when the person asks for several browsers, open one per source or subtopic and work them in parallel (several tool calls in one turn), so the job is done fast; read pages in the light browsers, real Chrome only where a login or a heavy web app needs it. Your browsers start with the person's accounts (the sites they signed in to for Hex) when there are any, in a light browser; pass `signedIn: true` on the call that opens one for heavy web apps or logins that need real Chrome, or `signedIn: false` for a clean one with no accounts. Never ask the person to sign in unless a page still shows a login after that. If a page shows a captcha or a Cloudflare challenge, retry it in a `signedIn: true` browser. If it still asks you to prove you are human, never try to solve or get around it yourself: leave that browser open on the challenge, do whatever else needs no such page, then ask the person to solve it, starting your reply with the line `HEX_QUESTION:` (one or two speakable sentences naming the site, saying they can solve it in the live picture of that browser on your card: Take over, solve it, Hand back). When they answer, look at the page afresh and go on.\n",
            crate::assistant::browser::max_sessions()
        ));
    }
    let Some(desktop) = desktop else {
        prompt.push_str(
            "You have no access to the desktop or other apps in this conversation: answer from what you know and what the person gave you. Do not offer to click, type, or open apps.\n",
        );
        return prompt;
    };
    prompt.push_str(&format!(
        "\nYou can see and use the {}'s apps through agent-desktop's tools (desktop_snapshot, desktop_click, desktop_list_apps, …). The guide below shows them as shell commands: each command is the tool of the same name (`agent-desktop list-apps` is desktop_list_apps) and its flags are the tool's arguments. Always call the tools; never run agent-desktop in a shell. Your session and agent id are already set.\n",
        computer_name()
    ));
    if desktop.guarded {
        prompt.push_str("Use hex_installed_apps to see what's installed before choosing an app.\n");
        prompt.push_str(
            "Every desktop action passes Hex's guard, which lets you act only in windows that are yours. A refusal comes back as a tool error with a code: NOT_YOUR_WINDOW (another task's window, or one you haven't launched or claimed), PERSON_IN_WINDOW (the person is working there), TAKEN_OVER (the person took this task over; look again before acting), POLICY_DENIED (a blocked app, or an action tasks may not do). Don't try to get around a refusal: choose another way, or say what you couldn't do.\n",
        );
    }
    if desktop.headed {
        prompt.push_str("Physical input (--headed) is allowed, but prefer headless actions; use physical input only when no semantic action exists.\n");
    } else {
        prompt.push_str("Use headless actions only: physical input is turned off, so actions that need --headed (hover, drag, mouse clicks at a point, double-click) are refused. Never try to switch to physical input.\n");
    }
    prompt.push_str(
        "Rules:\n\
         - Only do what the person asked, and do it: act instead of describing how or offering to. Never ask back what you can find out yourself (the message's desktop section, a snapshot, hex_installed_apps); if something only the person can tell is missing and there is no sensible default, ask for it (a job: in one message starting with `HEX_QUESTION:`) instead of guessing. Sending, deleting, paying, and posting follow the autonomy rules below.\n\
         - Which apps run and which windows are open: the message's \u{201c}desktop right now\u{201d} section is read fresh for this message; trust it over anything said earlier in the conversation or in other tasks' results (an app said to be open earlier may have been closed since). Use its window ids directly; list windows again only after something changed.\n\
         - Never quit or close an app you didn't open in this task.\n\
         - Read before acting: one snapshot of the window (interactive refs), act on a ref, then check only what matters.\n",
    );
    // Hex's app tools reach every app wherever the task may open apps.
    if desktop.guarded
        && (origin.lane() == Lane::Background || desktop.open_apps || desktop.front_window)
    {
        prompt.push_str(APP_METHODS);
    }
    match origin.lane() {
        Lane::Here if desktop.front_window => prompt.push_str(
            "- The person gave you full control of this Mac: look at and act in any window no other task holds, the one in front included, by its window_id (it becomes yours as you act in it; no need to claim it first) and open or switch to any app with hex_open_app (it takes a running app's window into this task, reopens one that has no window, and never starts a second instance). Never say you can't open, reach, or use an app or window, and never offer a background job for something you can do now. Asked only to open an app, open it, say so in one short sentence, and ask what to do in it.\n",
        ),
        Lane::Here if desktop.open_apps => prompt.push_str(
            "- The person wants their own windows left alone: act in the window in front only if asked about it, and never in other windows they use. But open or switch to any app with hex_open_app: its window becomes yours, parked out of their way and live on your card, and you act there right away (a running app is never started twice). Never say you can't open or use an app, and never offer a background job for something you can do now. Asked only to open an app, open it, say so in one short sentence, and ask what to do in it.\n",
        ),
        Lane::Here => prompt.push_str(
            "- You may LOOK at any window to answer: list them with desktop_list_windows and read or screenshot the one the person means by its window_id (\"what am I doing in Citrix?\" means the Citrix window, even if it isn't in front). ACT only in the window in front, named in the message, and only if asked; don't bring another window forward.\n- If the request needs to act anywhere beyond that window (take over or operate another window such as Citrix, open an app or a browser, the web, files), never say you can't and never tell the person to do it themselves: hand it off at once with `HEX_JOB:` with the hex_start_jobs tool (see below), however small it is; a background job may open apps and browsers and take over the person's other windows.\n",
        ),
        Lane::Background => prompt.push_str(&format!(
            "- You run beside the person and other agents, each with its own cursor; yours is agent “{}”. Work only in your own windows: to use any app, even one that already runs, call hex_open_app first; it takes the app's window into this task (never a second instance), parks it out of the person's way, and shows it live on your card. Never only check that an app runs and report that. Or claim a free window that is already open with desktop_claim_window (its window_id from desktop_list_windows). {}Never another task's windows: don't move, close, or type into them. The clipboard is Here-only: don't use it. Once you know a window's id, always pass it (--window-id / window_id), and don't bring windows to the front.\n\
             - {}\n",
            desktop.agent_id,
            if desktop.front_window {
                "The person allowed you to use the window in front of them too: claim it like any other when the job needs it (for example their browser tab or document), act in it without bringing other windows forward, and leave it as you found it apart from what the job changes. "
            } else {
                "Never the person's window (the one in front). "
            },
            if browser {
                "Use the browser tools for anything on the web; use the desktop only for native apps."
            } else {
                "For web work, open your own browser window rather than using one the person has open."
            }
        )),
    }
    if desktop.open_apps || desktop.front_window {
        prompt.push_str(
            "- Background first: act in windows where they are, parked or behind others; never bring one to the front just to act in it. Only an app that doesn't react in the background after one retry gets its window forward for the shortest time (the person sees the frame); check the result, then give the front back to the window the person was in.\n",
        );
    }
    if let Some(guide) = guide(desktop.executable()) {
        prompt.push_str("\nagent-desktop's guide:\n\n");
        prompt.push_str(guide);
    }
    prompt
}

/// agent-desktop's own guide (`skills get desktop` and `platform`), read
/// once per run of Hex.
fn guide(executable: &std::path::Path) -> Option<&'static str> {
    static GUIDE: OnceLock<Option<String>> = OnceLock::new();
    GUIDE
        .get_or_init(|| {
            let read = |args: &[&str]| -> Option<String> {
                let output = crate::assistant::backends::agent_command(executable)
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output()
                    .ok()?;
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
                value
                    .pointer("/data/content")
                    .and_then(serde_json::Value::as_str)
                    .map(strip_front_matter)
            };
            let desktop = read(&["skills", "get", "desktop"])?;
            let platform = read(&["skills", "get", "platform"]).unwrap_or_default();
            Some(trim_guide(&format!("{desktop}\n\n{platform}")))
        })
        .as_deref()
}

/// Sections of agent-desktop's guide written for a person or agent at a
/// shell, which a task never needs: it calls the tools (no shell, no
/// `--help`, no `skills get`), and Hex sets up the permissions.
const GUIDE_SKIPPED: &[&str] = &[
    "Shell syntax",
    "First run",
    "References",
    "Requirements",
    "First contact",
];

/// The guide without [`GUIDE_SKIPPED`]'s `##` sections, every one up to
/// the next heading. A guide whose headings changed loses nothing.
fn trim_guide(markdown: &str) -> String {
    let mut kept = String::with_capacity(markdown.len());
    let mut skipping = false;
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && line.starts_with('#') {
            skipping = line
                .strip_prefix("## ")
                .is_some_and(|title| GUIDE_SKIPPED.contains(&title.trim()));
        }
        if !skipping {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept.trim().to_owned()
}

/// Markdown without its leading `---` front matter.
fn strip_front_matter(markdown: &str) -> String {
    let trimmed = markdown.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        return rest[end + 4..].trim_start().to_owned();
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests;
