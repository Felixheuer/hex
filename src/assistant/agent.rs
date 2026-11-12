//! One task from start to end: builds its prompt (the question, its lane's
//! rules, a Here question's window and context, what else is going on, the
//! desktop's guide), gives it its tools (the guard's desktop server, or
//! agent-desktop directly when there is no guard, and a hidden browser for
//! Background work), runs it on the chosen backend (`provider`), reports
//! steps and windows, and saves the conversation as it goes, so a restart
//! finds it `Running` and offers to resume it.
//!
//! OWNER: brain.

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
}

/// What a restarted task is told before anything else.
pub const RESUME_NOTE: &str = "You were interrupted by a restart. Check what was already done (take a fresh look) before continuing; never repeat a send/submit you may already have done.";

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
    let asks = matches!(job.kind, JobKind::Ask | JobKind::Steer { .. });
    // What came before this run's own message.
    let mut history = stored.conversation.messages.clone();
    let mut image = None;
    if asks {
        let mut parts = vec![Part::Text {
            text: request.text.clone(),
        }];
        if let Some(screenshot) = &request.context.screenshot {
            match store.save_attachment(id, &screenshot.png) {
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
    if let Err(error) = store.save(&stored) {
        return Outcome::Failed(format!("{error:#}"));
    }
    emit(Event::ConversationsChanged);

    let backend = job.settings.backend;
    let key = provider::session_key(backend);
    let resume = stored.sessions.get(key).cloned();
    let mut run = match prepare(job, &history, resume.clone(), image) {
        Ok(run) => run,
        Err(message) => return finish(store, &mut stored, Vec::new(), Err(message), emit),
    };

    let mut seen = Seen::default();
    emit(Event::Thinking {
        conversation: id.clone(),
    });
    let mut result = run_backend(&run, control, id, emit, &mut seen, store, &mut stored);
    // A saved session the backend no longer has: start a new one that is
    // told the conversation so far.
    if let Err(error) = &result
        && resume.is_some()
        && !seen.streamed
        && seen.actions.is_empty()
        && !control.cancelled()
        && limit::detect(&format!("{error:#}")).is_none()
    {
        tracing::info!(error = %format!("{error:#}"), "the saved session didn't continue; starting a new one");
        run.resume = None;
        stored.sessions.remove(key);
        run.prompt = prompt(job, &history, true);
        result = run_backend(&run, control, id, emit, &mut seen, store, &mut stored);
    }
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
    finish(store, &mut stored, actions, result, emit)
}

/// What a backend run reported that the conversation keeps.
#[derive(Default)]
struct Seen {
    /// Desktop actions done, as the transcript records them.
    actions: Vec<String>,
    streamed: bool,
}

/// Runs the backend once, turning its reports into events, and saves its
/// session the moment it has one, so a restart can resume it.
fn run_backend(
    run: &Run,
    control: &Control,
    id: &ConversationId,
    emit: &mut dyn FnMut(Event),
    seen: &mut Seen,
    store: &Store,
    stored: &mut Stored,
) -> color_eyre::Result<provider::Finished> {
    let key = provider::session_key(run.backend);
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
            emit(Event::AnswerDelta {
                conversation: id.clone(),
                text,
            });
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
        AssistantBackend::OpenCode => settings.opencode_model.clone(),
        AssistantBackend::ClaudeApi => fast.or_else(|| Some(settings.claude_api_model.clone())),
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
                "agent-desktop isn't installed, so the assistant can't use apps. Install it in Labs › Assistant, or turn off “Use apps”.",
            )?,
        )
    } else {
        None
    };
    let lane = spec.lane;
    let browser = (settings.background_browser && lane == Lane::Background)
        .then(|| super::browser::mcp_server(&job.conversation))
        .flatten();
    let workdir = workdir()
        .map_err(|error| format!("Hex can't prepare the assistant's folder: {error:#}"))?;
    let origin = job.request.origin;
    let continued = resume.is_none() && settings.backend != AssistantBackend::ClaudeApi;
    let keep_warm = (lane == Lane::Here
        && settings.backend == AssistantBackend::ClaudeCode
        && settings.follow_up_seconds > 0)
        .then(|| Duration::from_secs(settings.follow_up_seconds.into()));
    Ok(Run {
        backend: settings.backend,
        model: model(job),
        system: system_prompt(origin, desktop.as_ref(), browser.is_some()),
        prompt: prompt(job, history, continued),
        image,
        resume,
        history: history.to_vec(),
        desktop,
        browser,
        workdir,
        quick: lane == Lane::Here,
        keep_warm,
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
                .get(provider::session_key(job.settings.backend))
                .cloned()
        });
    match prepare(job, &[], resume, None) {
        Ok(run) => provider::prewarm(&run),
        Err(why) => tracing::debug!(why, "nothing to start ahead"),
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
        seen.push(format!(
            "Your only window, the one in front: {}{id}",
            window.app
        ));
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
    }
    prompt
}

/// Who the assistant is, how it answers, and its lane's rules for the
/// desktop and the browser.
pub fn system_prompt(origin: Origin, desktop: Option<&Desktop>, browser: bool) -> String {
    let mut prompt = String::from(
        "You are Hex's assistant on the person's Mac. Hex is a voice app: the person speaks a request and you answer or do it.\n\
         Answer in the language the person used.\n",
    );
    prompt.push_str(match origin {
        Origin::ActionShortcut => {
            "This is a quick spoken question about the window in front of the person. Your answer is shown in a small bubble and read aloud, so keep it short (one to three sentences), plain, and speakable: no markdown, lists, tables, code blocks, links, or emoji. The question was transcribed from speech, so read past small transcription errors.\n"
        }
        Origin::Background => {
            "This is a background task, running beside the person and other tasks while they keep working. Do it, then end with one or two plain sentences saying what you did; no markdown.\n"
        }
        Origin::Chat => {
            "This is a typed chat; it may run beside other tasks. Markdown is fine; be as brief as the question allows.\n"
        }
    });
    if browser {
        prompt.push_str(
            "You have your own hidden browser (the browser tools): prefer it for anything on the web (looking things up, web apps, forms), over the person's browser windows.\n",
        );
    }
    let Some(desktop) = desktop else {
        prompt.push_str(
            "You have no access to the desktop or other apps in this conversation: answer from what you know and what the person gave you. Do not offer to click, type, or open apps.\n",
        );
        return prompt;
    };
    prompt.push_str(
        "\nYou can see and use the Mac's apps through agent-desktop's tools (desktop_snapshot, desktop_click, desktop_list_apps, …). The guide below shows them as shell commands: each command is the tool of the same name (`agent-desktop list-apps` is desktop_list_apps) and its flags are the tool's arguments. Always call the tools; never run agent-desktop in a shell. Your session and agent id are already set.\n",
    );
    if desktop.guarded {
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
         - Only do what the person asked. Ask nothing back; if something is ambiguous, risky, or irreversible (sending, deleting, paying, posting), stop and say what you would do instead.\n\
         - Never quit or close an app you didn't open in this task.\n\
         - Read before acting: snapshot or find, act on a ref, then verify.\n",
    );
    match origin.lane() {
        Lane::Here => prompt.push_str(
            "- Work only in the window in front, named in the message: read it, and act in it only if asked. Never touch any other window or app, and don't bring another one forward.\n",
        ),
        Lane::Background => prompt.push_str(&format!(
            "- You run beside the person and other agents, each with its own cursor; yours is agent “{}”. Work only in your own windows: open one with desktop_launch (it becomes yours), or take a free one that is already open with desktop_claim_window (its window_id from desktop_list_windows). Never touch the person's window (the one in front) or another task's windows; never move, close, or type into them. Once you know a window's id, always pass it (--window-id / window_id), and don't bring windows to the front.\n\
             - {}\n",
            desktop.agent_id,
            if browser {
                "Use the browser tools for anything on the web; use the desktop only for native apps."
            } else {
                "For web work, open your own browser window rather than using one the person has open."
            }
        )),
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
            Some(format!("{desktop}\n\n{platform}").trim().to_owned())
        })
        .as_deref()
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
