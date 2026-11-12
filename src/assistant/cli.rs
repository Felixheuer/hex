//! `hex assistant` (hidden): runs one assistant task here, without the app,
//! and prints its events as they come; lists and shows saved conversations.
//! For trying backends end to end. It runs even while Labs › Assistant is
//! off, because a person runs it on purpose.

use std::sync::mpsc;

use clap::{Subcommand, ValueEnum};
use color_eyre::{Result, eyre::eyre};

use super::service;
use super::types::{Context, Event, Origin, Part, Request, Role};
use crate::app_settings::AssistantBackend;

#[derive(Subcommand, Debug)]
pub enum AssistantCommand {
    /// Ask one question and print what happens until it's answered.
    Ask {
        text: String,
        /// The agent to use; the saved choice by default.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
        /// The backend's model; the saved one by default.
        #[arg(long)]
        model: Option<String>,
        /// Run it as an Agent mode task, in its own desktop session.
        #[arg(long, conflicts_with = "chat")]
        agent: bool,
        /// Run it as a typed Chats message, not a quick question.
        #[arg(long)]
        chat: bool,
        /// Don't let it use apps (no agent-desktop).
        #[arg(long)]
        no_desktop: bool,
        /// Read a quick answer aloud.
        #[arg(long)]
        speak: bool,
        /// Continue this conversation.
        #[arg(long = "continue", value_name = "ID")]
        conversation: Option<String>,
    },
    /// List saved conversations, newest first.
    Conversations,
    /// Print one saved conversation.
    Show { id: String },
    /// Save the Anthropic API key read from standard input in the Keychain
    /// (an empty input deletes it).
    Key,
    /// The guard's MCP server for one task (Hex gives it to each task's
    /// agent in place of agent-desktop): `--spec` is the task's JSON spec or
    /// a file holding it.
    #[command(hide = true)]
    Mcp {
        #[arg(long)]
        spec: String,
    },
    /// A task's hidden browser as an MCP server on stdio (agent-browser's
    /// own, pinned to the task's session). Started by the task's agent.
    #[command(hide = true)]
    BrowserMcp {
        #[arg(long)]
        task: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Backend {
    ClaudeCode,
    Codex,
    #[value(name = "opencode")]
    OpenCode,
    ClaudeApi,
}

impl From<Backend> for AssistantBackend {
    fn from(backend: Backend) -> Self {
        match backend {
            Backend::ClaudeCode => Self::ClaudeCode,
            Backend::Codex => Self::Codex,
            Backend::OpenCode => Self::OpenCode,
            Backend::ClaudeApi => Self::ClaudeApi,
        }
    }
}

/// Runs `command`; the process's exit code.
pub fn run(command: AssistantCommand) -> u8 {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("HEX_ASSISTANT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();
    let result = match command {
        AssistantCommand::Ask {
            text,
            backend,
            model,
            agent,
            chat,
            no_desktop,
            speak,
            conversation,
        } => ask(Ask {
            text,
            backend: backend.map(Into::into),
            model,
            origin: if agent {
                Origin::Background
            } else if chat {
                Origin::Chat
            } else {
                Origin::ActionShortcut
            },
            no_desktop,
            speak,
            conversation,
        }),
        AssistantCommand::Conversations => {
            list();
            Ok(true)
        }
        AssistantCommand::Show { id } => show(&id).map(|()| true),
        AssistantCommand::Key => key().map(|()| true),
        AssistantCommand::Mcp { spec } => super::guard::serve(&spec).map(|()| true),
        AssistantCommand::BrowserMcp { task } => super::browser::serve(&task).map(|()| true),
    };
    match result {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(error) => {
            eprintln!("hex: {error:#}");
            1
        }
    }
}

struct Ask {
    text: String,
    backend: Option<AssistantBackend>,
    model: Option<String>,
    origin: Origin,
    no_desktop: bool,
    speak: bool,
    conversation: Option<String>,
}

/// Runs one task; whether it was answered.
fn ask(ask: Ask) -> Result<bool> {
    service::allow_without_lab();
    let Ask {
        backend,
        model,
        no_desktop,
        speak,
        ..
    } = ask;
    service::adjust_settings(move |settings| {
        if let Some(backend) = backend {
            settings.backend = backend;
        }
        if let Some(model) = &model {
            match settings.backend {
                AssistantBackend::ClaudeCode => settings.claude_code_model = Some(model.clone()),
                AssistantBackend::Codex => settings.codex_model = Some(model.clone()),
                AssistantBackend::OpenCode => settings.opencode_model = Some(model.clone()),
                AssistantBackend::ClaudeApi => settings.claude_api_model = model.clone(),
            }
        }
        if no_desktop {
            settings.control_desktop = false;
        }
        settings.speak_answers = speak;
    });
    let (sender, events) = mpsc::channel();
    service::subscribe(Box::new(move |event| {
        let _ = sender.send(event.clone());
    }));
    let _ = ctrlc::set_handler(service::cancel_all);
    if ask.origin == Origin::Background {
        service::set_agent_mode(true);
    }
    let id = service::submit(Request {
        conversation: ask.conversation,
        text: ask.text,
        origin: ask.origin,
        context: Context::default(),
    });
    println!("conversation {id}");
    let mut answered = false;
    let mut mid_line = false;
    for event in events {
        if event
            .conversation()
            .is_some_and(|conversation| *conversation != id)
        {
            continue;
        }
        if mid_line && !matches!(event, Event::AnswerDelta { .. }) {
            println!();
            mid_line = false;
        }
        #[allow(clippy::wildcard_enum_match_arm)]
        match &event {
            Event::Paused { .. } => println!("[paused]"),
            Event::Resumed { .. } => println!("[resumed]"),
            Event::Waiting { reason, .. } => println!("[waiting] {reason}"),
            Event::HandsOff { on, .. } => println!("[hands off] {on}"),
            Event::Queued { .. } => println!("[queued]"),
            Event::Started { origin, .. } => println!("[started] {origin:?}"),
            Event::Thinking { .. } => println!("[thinking]"),
            Event::Step { description, .. } => println!("[step] {description}"),
            Event::Window { window, .. } => match &window.window_id {
                Some(id) => println!("[window] {} ({id})", window.app),
                None => println!("[window] {}", window.app),
            },
            Event::AnswerDelta { text, .. } => {
                if !mid_line {
                    print!("[delta] ");
                }
                print!("{text}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                mid_line = true;
            }
            Event::Answer { text, .. } => {
                println!("[answer] {text}");
                answered = true;
            }
            Event::Failed { message, .. } => println!("[failed] {message}"),
            Event::Cancelled { .. } => println!("[cancelled]"),
            Event::ConversationsChanged => println!("[saved]"),
            Event::ModeChanged { on } => {
                println!("[agent mode {}]", if *on { "on" } else { "off" })
            }
        }
        if event.is_final() {
            break;
        }
    }
    if ask.origin == Origin::Background {
        service::set_agent_mode(false);
    }
    Ok(answered)
}

fn list() {
    for summary in service::conversations() {
        println!(
            "{}  {:<14} {}  {}",
            summary.id,
            format!("{:?}", summary.origin),
            summary.title,
            summary.preview
        );
    }
}

fn show(id: &str) -> Result<()> {
    let conversation = service::conversation(id).ok_or_else(|| eyre!("no conversation “{id}”"))?;
    println!("{} ({:?})", conversation.title, conversation.origin);
    for message in &conversation.messages {
        let who = if message.role == Role::User {
            "You"
        } else {
            "Assistant"
        };
        for part in &message.parts {
            match part {
                Part::Text { text } => println!("{who}: {text}"),
                Part::Screenshot { file } => println!("{who}: [screenshot {file}]"),
                Part::Action { description } => println!("{who}: [{description}]"),
                Part::Error { message } => println!("{who}: [error] {message}"),
            }
        }
    }
    Ok(())
}

fn key() -> Result<()> {
    let mut key = String::new();
    std::io::stdin().read_line(&mut key)?;
    super::credentials::set_api_key(&key)?;
    println!(
        "{}",
        if key.trim().is_empty() {
            "The saved API key was deleted."
        } else {
            "The API key is saved in the Keychain."
        }
    );
    Ok(())
}
