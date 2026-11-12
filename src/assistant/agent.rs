//! One task from start to end: builds its prompt (the question, a quick
//! question's context, agent-desktop's guide, its agent id and session),
//! runs it on the chosen backend (`provider`), reports steps and windows,
//! and saves the conversation.
//!
//! OWNER: brain.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::app_settings::{AssistantBackend, AssistantSettings};

use super::provider::{self, Control, Desktop, Report, Run};
use super::store::{self, Store, Stored};
use super::types::{ConversationId, Event, Message, Origin, Part, Request, Role};

/// One task as the runner hands it over.
#[derive(Clone, Debug)]
pub struct Job {
    pub conversation: ConversationId,
    pub request: Request,
    pub settings: AssistantSettings,
    /// This task's agent id in the desktop session: its own cursor.
    pub agent_id: String,
    /// Agent mode's multi-agent session, when one is running.
    pub desktop_session: Option<String>,
}

/// How a task ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Answer(String),
    Failed(String),
    Cancelled,
}

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
    let history = stored.conversation.messages.clone();

    let mut parts = vec![Part::Text {
        text: request.text.clone(),
    }];
    let mut image = None;
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
    let mut result = run_backend(&run, control, id, emit, &mut seen);
    // A saved session the backend no longer has: start a new one that is
    // told the conversation so far.
    if let Err(error) = &result
        && resume.is_some()
        && !seen.streamed
        && seen.actions.is_empty()
        && !control.cancelled()
    {
        tracing::info!(error = %format!("{error:#}"), "the saved session didn't continue; starting a new one");
        run.resume = None;
        run.prompt = prompt(request, &history, true);
        result = run_backend(&run, control, id, emit, &mut seen);
    }
    if let Some(session) = seen.session.take() {
        stored.sessions.insert(key.to_owned(), session);
    }
    let actions = std::mem::take(&mut seen.actions);
    if control.cancelled() {
        let _ = finish(store, &mut stored, actions, Err("Stopped.".into()), emit);
        return Outcome::Cancelled;
    }
    let result = result
        .map(|finished| finished.text)
        .map_err(|error| format!("{error:#}"));
    finish(store, &mut stored, actions, result, emit)
}

/// What a backend run reported that the conversation keeps.
#[derive(Default)]
struct Seen {
    /// Desktop actions done, as the transcript records them.
    actions: Vec<String>,
    streamed: bool,
    session: Option<String>,
}

/// Runs the backend once, turning its reports into events.
fn run_backend(
    run: &Run,
    control: &Control,
    id: &ConversationId,
    emit: &mut dyn FnMut(Event),
    seen: &mut Seen,
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
            emit(Event::AnswerDelta {
                conversation: id.clone(),
                text,
            });
        }
        Report::Session(session) => seen.session = Some(session),
    };
    provider::run(run, control, &mut report)
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
    if let Err(error) = store.save(stored) {
        tracing::warn!(%error, "couldn't save the assistant's answer");
    }
    emit(Event::ConversationsChanged);
    outcome
}

/// The backend run for `job`, or why it can't run.
fn prepare(
    job: &Job,
    history: &[Message],
    resume: Option<String>,
    image: Option<PathBuf>,
) -> Result<Run, String> {
    let settings = &job.settings;
    let desktop = if settings.control_desktop {
        Some(
            Desktop::new(
                job.desktop_session.clone(),
                job.agent_id.clone(),
                settings.physical_input,
            )
            .ok_or(
                "agent-desktop isn't installed, so the assistant can't use apps. Install it in Labs › Assistant, or turn off “Use apps”.",
            )?,
        )
    } else {
        None
    };
    let workdir = workdir()
        .map_err(|error| format!("Hex can't prepare the assistant's folder: {error:#}"))?;
    let origin = job.request.origin;
    let model = match settings.backend {
        AssistantBackend::ClaudeCode => settings.claude_code_model.clone(),
        AssistantBackend::Codex => settings.codex_model.clone(),
        AssistantBackend::OpenCode => settings.opencode_model.clone(),
        AssistantBackend::ClaudeApi => Some(settings.claude_api_model.clone()),
    };
    let continued = resume.is_none() && settings.backend != AssistantBackend::ClaudeApi;
    Ok(Run {
        backend: settings.backend,
        model,
        system: system_prompt(origin, desktop.as_ref()),
        prompt: prompt(&job.request, history, continued),
        image,
        resume,
        history: history.to_vec(),
        desktop,
        workdir,
        quick: origin == Origin::ActionShortcut,
    })
}

/// The folder every backend runs in: private, outside any project, and the
/// same each time so a backend finds its saved sessions.
fn workdir() -> color_eyre::Result<PathBuf> {
    let dir = store::assistant_dir()?.join("workspace");
    crate::app_paths::create_private_dir(&dir)?;
    Ok(dir)
}

/// The first message: the conversation so far when the backend has no
/// session for it (`with_history`), what was on screen for a quick
/// question, then the question.
pub fn prompt(request: &Request, history: &[Message], with_history: bool) -> String {
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
    if !prompt.is_empty() {
        prompt.push_str("The person asks: ");
    }
    prompt.push_str(request.text.trim());
    prompt
}

/// Who the assistant is, how it answers, and the rules for the desktop.
pub fn system_prompt(origin: Origin, desktop: Option<&Desktop>) -> String {
    let mut prompt = String::from(
        "You are Hex's assistant on the person's Mac. Hex is a voice app: the person speaks a request and you answer or do it.\n\
         Answer in the language the person used.\n",
    );
    prompt.push_str(match origin {
        Origin::ActionShortcut => {
            "This is a quick spoken question. Your answer is shown in a small bubble and read aloud, so keep it short (one to three sentences), plain, and speakable: no markdown, lists, tables, code blocks, links, or emoji. The question was transcribed from speech, so read past small transcription errors.\n"
        }
        Origin::AgentMode => {
            "This is one task in Agent mode, running beside other tasks. Do it on the desktop, then end with one or two plain sentences saying what you did; no markdown.\n"
        }
        Origin::Chat => {
            "This is a typed chat. Markdown is fine; be as brief as the question allows.\n"
        }
    });
    let Some(desktop) = desktop else {
        prompt.push_str(
            "You have no access to the desktop or other apps in this conversation: answer from what you know and what the person gave you. Do not offer to click, type, or open apps.\n",
        );
        return prompt;
    };
    prompt.push_str(
        "\nYou can see and use the Mac's apps through agent-desktop's tools (desktop_snapshot, desktop_click, desktop_list_apps, …). The guide below shows them as shell commands: each command is the tool of the same name (`agent-desktop list-apps` is desktop_list_apps) and its flags are the tool's arguments. Always call the tools; never run agent-desktop in a shell. Your session and agent id are already set.\n",
    );
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
    if origin == Origin::AgentMode || desktop.session.is_some() {
        prompt.push_str(&format!(
            "- Other agents are working on this Mac at the same time, each with its own cursor; yours is agent “{}”. Work only in windows you opened or were asked to use. Once you know a window's id, always pass it (--window-id / window_id). Never touch, move, close, or type into another task's windows, and don't change the frontmost app unless you must.\n",
            desktop.agent_id
        ));
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
mod tests {
    use super::*;
    use crate::assistant::types::Context;

    fn request(text: &str, context: Context) -> Request {
        Request {
            conversation: None,
            text: text.into(),
            origin: Origin::ActionShortcut,
            context,
        }
    }

    #[test]
    fn a_quick_question_carries_what_was_on_screen() {
        let context = Context {
            screenshot: None,
            selected_text: Some(" the invoice total ".into()),
            frontmost_app: Some("Mail".into()),
            window_title: Some("Inbox".into()),
        };
        let prompt = prompt(&request("What's this?", context), &[], false);
        assert_eq!(
            prompt,
            "What the person has on screen as they ask:\nFrontmost app: Mail\nIts window: Inbox\nSelected text:\n<<<\nthe invoice total\n>>>\n\nThe person asks: What's this?"
        );
        assert_eq!(prompt_of("Hi"), "Hi");
    }

    fn prompt_of(text: &str) -> String {
        prompt(&request(text, Context::default()), &[], true)
    }

    #[test]
    fn a_new_session_is_told_the_conversation_so_far() {
        let history = [
            Message {
                role: Role::User,
                parts: vec![Part::Text {
                    text: "2+2?".into(),
                }],
                at_ms: 1,
            },
            Message {
                role: Role::Assistant,
                parts: vec![Part::Text { text: "4".into() }],
                at_ms: 2,
            },
        ];
        let prompt = prompt(&request("And times 3?", Context::default()), &history, true);
        assert_eq!(
            prompt,
            "The conversation so far:\nPerson: 2+2?\nYou: 4\n\nThe person asks: And times 3?"
        );
    }

    #[test]
    fn without_the_desktop_the_prompt_forbids_it() {
        let quick = system_prompt(Origin::ActionShortcut, None);
        assert!(quick.contains("no markdown"));
        assert!(quick.contains("no access to the desktop"));
        let chat = system_prompt(Origin::Chat, None);
        assert!(chat.contains("Markdown is fine"));
    }

    #[test]
    fn front_matter_is_dropped_from_the_guide() {
        assert_eq!(
            strip_front_matter("---\nname: x\n---\n\n# Guide"),
            "# Guide"
        );
        assert_eq!(strip_front_matter("# Guide"), "# Guide");
    }
}
