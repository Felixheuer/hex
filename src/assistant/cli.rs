//! `hex assistant` (hidden): runs one assistant task here, without the app,
//! and prints its events as they come (with how long the first token and
//! the first spoken sentence took); lists and shows saved conversations;
//! resumes an interrupted one; and asks a running Hex to stop all tasks or
//! to let the person take one over. For trying backends end to end. It runs
//! even while the assistant is off, because a person runs it on purpose.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Subcommand, ValueEnum};
use color_eyre::{Result, eyre::eyre};

use super::service::{self, remote};
use super::types::{Context, Event, Lane, Origin, Part, Request, Role, TaskWindow};
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
        /// Its lane: `here` (the window in front; the default) or
        /// `background` (its own windows, beside other tasks).
        #[arg(long, value_enum, conflicts_with = "chat")]
        lane: Option<LaneArg>,
        /// Same as `--lane background`.
        #[arg(long, conflicts_with_all = ["chat", "lane"])]
        agent: bool,
        /// Run it as a typed Chats message, not a quick question.
        #[arg(long)]
        chat: bool,
        /// The Here lane's window: the app in front (agent-desktop's name).
        #[arg(long)]
        window_app: Option<String>,
        /// The Here lane's window id (agent-desktop's).
        #[arg(long, requires = "window_app")]
        window_id: Option<String>,
        /// Start the backend this many seconds before asking, as the
        /// Action shortcut does while the person speaks.
        #[arg(long, value_name = "SECONDS")]
        prewarm: Option<f64>,
        /// Then ask this follow-up in the same conversation.
        #[arg(long, value_name = "TEXT")]
        then: Option<String>,
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
    /// Continue a conversation Hex quit in the middle of, and print what
    /// happens.
    Resume { id: String },
    /// Ask the running Hex to stop every assistant task.
    StopAll,
    /// Ask the running Hex to pause a task and bring its window forward.
    TakeOver { id: String },
    /// Ask the running Hex to give a task back after `take-over`.
    HandBack { id: String },
    /// List, add, remove, or watch scheduled jobs.
    Schedules {
        #[command(subcommand)]
        command: Option<SchedulesCommand>,
    },
    /// Hex's own tools for a task's agent (memory, schedules) as an MCP
    /// server on stdio. Started by the task's agent.
    #[command(hide = true)]
    ToolsMcp {
        #[arg(long)]
        task: String,
    },
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
    /// Closes every hidden browser a task opened.
    #[command(hide = true)]
    BrowserClose {
        #[arg(long)]
        task: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SchedulesCommand {
    /// Every schedule with its next run (the default).
    List,
    /// Schedule a job: `--when "0 8 * * 1-5"` or an RRULE.
    Add {
        #[arg(long)]
        when: String,
        /// What to do each time.
        task: String,
    },
    /// Remove a schedule.
    Remove {
        id: String,
    },
    /// Turn a schedule on or off.
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Run due schedules here for a while, printing what happens (for
    /// trying them without the app).
    Watch {
        #[arg(long, default_value_t = 2.0)]
        minutes: f64,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LaneArg {
    Here,
    Background,
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
            lane,
            agent,
            chat,
            window_app,
            window_id,
            prewarm,
            then,
            no_desktop,
            speak,
            conversation,
        } => ask(Ask {
            text,
            backend: backend.map(Into::into),
            model,
            origin: if agent || matches!(lane, Some(LaneArg::Background)) {
                Origin::Background
            } else if chat {
                Origin::Chat
            } else {
                Origin::ActionShortcut
            },
            window: window_app.map(|app| TaskWindow { app, window_id }),
            prewarm: prewarm.map(Duration::from_secs_f64),
            then,
            no_desktop,
            speak,
            conversation,
        }),
        AssistantCommand::Resume { id } => resume(&id),
        AssistantCommand::StopAll => send(remote::ALL, remote::Verb::Stop),
        AssistantCommand::TakeOver { id } => send(&id, remote::Verb::TakeOver),
        AssistantCommand::HandBack { id } => send(&id, remote::Verb::HandBack),
        AssistantCommand::Schedules { command } => schedules(command),
        AssistantCommand::ToolsMcp { task } => super::tools::serve(&task).map(|()| true),
        AssistantCommand::Conversations => {
            list();
            Ok(true)
        }
        AssistantCommand::Show { id } => show(&id).map(|()| true),
        AssistantCommand::Key => key().map(|()| true),
        AssistantCommand::Mcp { spec } => super::guard::serve(&spec).map(|()| true),
        AssistantCommand::BrowserMcp { task } => super::browser::serve(&task).map(|()| true),
        AssistantCommand::BrowserClose { task } => {
            super::browser::close(&task);
            Ok(true)
        }
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
    window: Option<TaskWindow>,
    prewarm: Option<Duration>,
    then: Option<String>,
    no_desktop: bool,
    speak: bool,
    conversation: Option<String>,
}

/// Runs one task (and its follow-up); whether it was answered.
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
            settings.here_model = Some(model.clone());
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
    let _ = ctrlc::set_handler(service::stop_all);
    let lane = ask.origin.lane();
    // A Here question is about the window in front, read as the Action
    // shortcut reads it (Accessibility only, no screenshot).
    let context = match &ask.window {
        Some(window) => Context {
            frontmost_app: Some(window.app.clone()),
            window: Some(window.clone()),
            ..Context::default()
        },
        None if lane == Lane::Here => super::computer::quick_context(),
        None => Context::default(),
    };
    let window = context.window.clone();
    if let Some(window) = &window {
        println!("[window in front] {} {:?}", window.app, window.window_id);
    }
    if let Some(ahead) = ask.prewarm {
        service::prewarm_for(lane, window.clone());
        std::thread::sleep(ahead);
    }
    let id = service::submit(Request {
        conversation: ask.conversation,
        text: ask.text,
        origin: ask.origin,
        context: context.clone(),
    });
    println!("conversation {id}");
    let mut answered = follow(&id, &events, speak);
    if let Some(then) = ask.then.filter(|_| answered) {
        if let Some(ahead) = ask.prewarm {
            service::prewarm_for(lane, window.clone());
            std::thread::sleep(ahead);
        }
        let again = service::submit(Request {
            conversation: Some(id.clone()),
            text: then,
            origin: ask.origin,
            context,
        });
        println!("follow-up {again}");
        answered = follow(&again, &events, speak);
    }
    service::stop_all();
    Ok(answered)
}

/// Prints `id`'s events until it ends, with timings; whether it was
/// answered. When the answer is read aloud, waits for it to be read.
fn follow(id: &str, events: &mpsc::Receiver<Event>, speak: bool) -> bool {
    let started = Instant::now();
    let mut first_token = None;
    let mut first_speech = None;
    let mut answered = false;
    let mut mid_line = false;
    for event in events {
        if event
            .conversation()
            .is_some_and(|conversation| conversation != id)
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
            Event::Waiting {
                reason, until_ms, ..
            } => {
                println!("[waiting until {until_ms}] {reason}")
            }
            Event::HandsOff { on, .. } => println!("[hands off] {on}"),
            Event::Plan { steps, .. } => {
                for step in steps {
                    println!(
                        "[plan] {} {}",
                        if step.done { "✓" } else { "·" },
                        step.title
                    );
                }
            }
            Event::Question { text, .. } => {
                println!("[question] {text}");
                // The answer: a line from standard input, as a hold would
                // give it.
                let mut answer = String::new();
                let _ = std::io::stdin().read_line(&mut answer);
                if answer.trim().is_empty() {
                    service::cancel(id);
                } else {
                    println!("[answering] {}", answer.trim());
                    service::submit(Request {
                        conversation: Some(id.to_owned()),
                        text: answer.trim().to_owned(),
                        origin: Origin::ActionShortcut,
                        context: Context::default(),
                    });
                }
            }
            Event::Escalated { .. } => println!("[escalated] continues in the background"),
            Event::Queued { .. } => println!("[queued]"),
            Event::Started { origin, .. } => println!("[started] {origin:?}"),
            Event::Thinking { .. } => println!("[thinking]"),
            Event::Step { description, .. } => println!("[step] {description}"),
            Event::Window { window, .. } => match &window.window_id {
                Some(id) => println!("[window] {} ({id})", window.app),
                None => println!("[window] {}", window.app),
            },
            Event::AnswerDelta { text, .. } => {
                first_token.get_or_insert_with(|| started.elapsed());
                if speak && first_speech.is_none() && super::speech::speaking() {
                    first_speech = Some(started.elapsed());
                }
                if !mid_line {
                    print!("[delta] ");
                }
                print!("{text}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                mid_line = true;
            }
            Event::Answer { text, .. } => {
                first_token.get_or_insert_with(|| started.elapsed());
                println!("[answer] {text}");
                answered = true;
            }
            Event::Failed { message, .. } => println!("[failed] {message}"),
            Event::Cancelled { .. } => println!("[cancelled]"),
            Event::ConversationsChanged => println!("[saved]"),
            Event::ModeChanged { on } => {
                println!("[frame {}]", if *on { "on" } else { "off" })
            }
        }
        if event.is_final() {
            break;
        }
    }
    if speak && answered {
        // Wait for the reading to start and end.
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if super::speech::speaking() {
                first_speech.get_or_insert_with(|| started.elapsed());
            } else if first_speech.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let ms = |time: Option<Duration>| {
        time.map_or_else(|| "-".to_owned(), |time| format!("{} ms", time.as_millis()))
    };
    println!(
        "[timing] first token {}, first spoken word {}, total {} ms",
        ms(first_token),
        ms(first_speech),
        started.elapsed().as_millis()
    );
    answered
}

/// Resumes an interrupted conversation and prints what happens.
fn resume(id: &str) -> Result<bool> {
    service::allow_without_lab();
    let (sender, events) = mpsc::channel();
    service::subscribe(Box::new(move |event| {
        let _ = sender.send(event.clone());
    }));
    let _ = ctrlc::set_handler(service::stop_all);
    let id = service::resume_interrupted(id)
        .ok_or_else(|| eyre!("“{id}” wasn't interrupted, or there is no such conversation"))?;
    println!("conversation {id}");
    Ok(follow(&id, &events, false))
}

fn schedules(command: Option<SchedulesCommand>) -> Result<bool> {
    use super::schedules;
    match command.unwrap_or(SchedulesCommand::List) {
        SchedulesCommand::List => {
            let list = schedules::list();
            if list.is_empty() {
                println!("No scheduled jobs.");
            }
            for schedule in list {
                let next = schedule
                    .next_due()
                    .map_or_else(|| "never".into(), schedules::describe);
                println!(
                    "{}  {:<3}  {:<24}  next {next}  {}",
                    schedule.id,
                    if schedule.enabled { "on" } else { "off" },
                    schedule.when,
                    schedule.task
                );
            }
            Ok(true)
        }
        SchedulesCommand::Add { when, task } => {
            let schedule = schedules::add(&when, &task, Lane::Background)?;
            let next: Vec<String> = schedules::next_runs(&schedule.when, 3)?
                .into_iter()
                .map(schedules::describe)
                .collect();
            println!("{}  next: {}", schedule.id, next.join(", "));
            Ok(true)
        }
        SchedulesCommand::Remove { id } => Ok(schedules::remove(&id)),
        SchedulesCommand::Enable { id } => Ok(schedules::set_enabled(&id, true)),
        SchedulesCommand::Disable { id } => Ok(schedules::set_enabled(&id, false)),
        SchedulesCommand::Watch { minutes } => {
            service::allow_without_lab();
            let (sender, events) = mpsc::channel();
            service::subscribe(Box::new(move |event| {
                let _ = sender.send(event.clone());
            }));
            let _ = ctrlc::set_handler(service::stop_all);
            let until = Instant::now() + Duration::from_secs_f64(minutes * 60.0);
            let mut ran = false;
            while Instant::now() < until {
                for id in service::run_due_schedules() {
                    println!("[scheduled run] conversation {id}");
                    ran |= follow(&id, &events, false);
                }
                std::thread::sleep(Duration::from_secs(5));
            }
            service::stop_all();
            Ok(ran)
        }
    }
}

/// Asks every running Hex to do `verb` to `target`.
fn send(target: &str, verb: remote::Verb) -> Result<bool> {
    remote::send(target, verb, Duration::from_secs(2))?;
    println!("Asked the running Hex; a Hex that runs no such task does nothing.");
    Ok(true)
}

fn list() {
    for summary in service::conversations() {
        println!(
            "{}  {:<26} {}  {}",
            summary.id,
            format!("{:?}/{:?}", summary.origin, summary.outcome),
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
